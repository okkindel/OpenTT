//! The built-in mixer: our sounds made from the game's (`How::Sound`)
//! when ffmpeg is not there. It runs the same filter graphs (the subset
//! `mapping.ron` uses: `asetrate`, `aresample`, `atrim`, `asetpts`,
//! `asplit`, `adelay`, `volume`, `amix`, `bass`, `highpass`, `lowpass`,
//! `afade`) on the decoded inputs, brings the result to the entry's level
//! (EBU R128 integrated loudness or sample peak) and writes 16-bit mono
//! WAV at 44.1 kHz. Not bit-identical to ffmpeg's Vorbis files (simpler
//! resampling and shelving), but the same sound at the same level.

use crate::mapping::Level;
use std::collections::HashMap;
use std::f64::consts::PI;

/// The output rate.
pub const RATE: u32 = 44100;

/// Mono samples at a rate.
#[derive(Debug, Clone, PartialEq)]
pub struct Signal {
    pub rate: u32,
    pub samples: Vec<f32>,
}

/// Makes the sound: `inputs` (the files' bytes, Ogg Vorbis or WAV) through
/// `graph` to `level`, as a WAV file.
pub fn render(inputs: &[Vec<u8>], graph: &str, level: Level) -> Result<Vec<u8>, String> {
    let decoded = inputs.iter().map(|b| decode(b)).collect::<Result<Vec<_>, _>>()?;
    let mut out = run_graph(&decoded, graph)?;
    if out.rate != RATE {
        out = resample(&out, RATE);
    }
    let gain_db = match level {
        Level::Lufs(target) => target as f64 - loudness(&out),
        Level::Peak(target) => target as f64 - peak_db(&out),
    };
    let g = 10f64.powf(gain_db / 20.0) as f32;
    for s in &mut out.samples {
        *s = (*s * g).clamp(-1.0, 1.0);
    }
    Ok(wav(&out))
}

/// Decodes Ogg Vorbis or 16-bit PCM WAV to mono.
pub fn decode(bytes: &[u8]) -> Result<Signal, String> {
    if bytes.starts_with(b"OggS") {
        let mut rd = lewton::inside_ogg::OggStreamReader::new(std::io::Cursor::new(bytes)).map_err(|e| format!("vorbis: {e}"))?;
        let ch = rd.ident_hdr.audio_channels.max(1) as usize;
        let rate = rd.ident_hdr.audio_sample_rate;
        let mut samples = Vec::new();
        while let Some(pck) = rd.read_dec_packet_itl().map_err(|e| format!("vorbis: {e}"))? {
            for frame in pck.chunks(ch) {
                samples.push(frame.iter().map(|&s| s as f32 / 32768.0).sum::<f32>() / ch as f32);
            }
        }
        return Ok(Signal { rate, samples });
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
        let (mut ch, mut rate, mut bits) = (1usize, RATE, 16u16);
        let mut at = 12;
        while at + 8 <= bytes.len() {
            let id = &bytes[at..at + 4];
            let len = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
            let body = &bytes[at + 8..(at + 8 + len).min(bytes.len())];
            if id == b"fmt " && body.len() >= 16 {
                ch = u16::from_le_bytes([body[2], body[3]]).max(1) as usize;
                rate = u32::from_le_bytes(body[4..8].try_into().unwrap());
                bits = u16::from_le_bytes([body[14], body[15]]);
            } else if id == b"data" {
                if bits != 16 {
                    return Err(format!("WAV: {bits}-bit samples are not supported"));
                }
                let samples = body.chunks_exact(2 * ch).map(|f| f.as_chunks::<2>().0.iter().map(|s| i16::from_le_bytes(*s) as f32 / 32768.0).sum::<f32>() / ch as f32).collect();
                return Ok(Signal { rate, samples });
            }
            at += 8 + len + (len & 1);
        }
        return Err("WAV: no data".into());
    }
    Err("unknown audio format (expected Ogg Vorbis or WAV)".into())
}

/// 16-bit mono PCM WAV.
pub fn wav(s: &Signal) -> Vec<u8> {
    let data = (s.samples.len() * 2) as u32;
    let mut v = Vec::with_capacity(44 + data as usize);
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(36 + data).to_le_bytes());
    v.extend_from_slice(b"WAVEfmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&s.rate.to_le_bytes());
    v.extend_from_slice(&(s.rate * 2).to_le_bytes());
    v.extend_from_slice(&2u16.to_le_bytes());
    v.extend_from_slice(&16u16.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&data.to_le_bytes());
    for x in &s.samples {
        v.extend_from_slice(&((x.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes());
    }
    v
}

/// Runs a filter graph (`[in]f1=..,f2[out];...`, inputs `[N:a]`) and
/// returns the stream labelled `out` (or the last one made).
pub fn run_graph(inputs: &[Signal], graph: &str) -> Result<Signal, String> {
    let mut streams: HashMap<String, Signal> = HashMap::new();
    for (i, s) in inputs.iter().enumerate() {
        streams.insert(format!("{i}:a"), s.clone());
    }
    let mut last = None;
    for chain in graph.split(';').map(str::trim).filter(|c| !c.is_empty()) {
        let (ins, rest) = labels_front(chain);
        let (body, outs) = labels_back(rest);
        let mut cur: Vec<Signal> = ins.iter().map(|l| streams.remove(l).ok_or_else(|| format!("filter graph: no stream [{l}]"))).collect::<Result<_, _>>()?;
        for f in body.split(',').map(str::trim).filter(|f| !f.is_empty()) {
            cur = filter(f, cur)?;
        }
        if outs.is_empty() {
            last = cur.pop();
        } else {
            if outs.len() != cur.len() {
                return Err(format!("filter graph: {} outputs for {} streams in {chain}", outs.len(), cur.len()));
            }
            for (l, s) in outs.into_iter().zip(cur) {
                last = Some(s.clone());
                streams.insert(l, s);
            }
        }
    }
    streams.remove("out").or(last).ok_or_else(|| "filter graph: no output".to_string())
}

/// Leading `[label]`s of a chain and the rest.
fn labels_front(s: &str) -> (Vec<String>, &str) {
    let mut v = Vec::new();
    let mut rest = s.trim_start();
    while let Some(r) = rest.strip_prefix('[') {
        let Some(end) = r.find(']') else { break };
        v.push(r[..end].to_string());
        rest = r[end + 1..].trim_start();
    }
    (v, rest)
}

/// Trailing `[label]`s of a chain and the rest.
fn labels_back(s: &str) -> (&str, Vec<String>) {
    let mut v = Vec::new();
    let mut rest = s.trim_end();
    while let Some(r) = rest.strip_suffix(']') {
        let Some(start) = r.rfind('[') else { break };
        v.insert(0, r[start + 1..].to_string());
        rest = r[..start].trim_end();
    }
    (rest, v)
}

/// A filter's options: `name=a:b:k=v` → positional and named values.
fn options(args: &str) -> (Vec<String>, HashMap<String, String>) {
    let mut pos = Vec::new();
    let mut named = HashMap::new();
    for a in args.split(':').filter(|a| !a.is_empty()) {
        match a.split_once('=') {
            Some((k, v)) => {
                named.insert(k.to_string(), v.to_string());
            }
            None => pos.push(a.to_string()),
        }
    }
    (pos, named)
}

fn num(v: Option<&String>, default: f64) -> Result<f64, String> {
    match v {
        None => Ok(default),
        Some(s) => {
            let t = s.trim();
            match t.strip_suffix("dB") {
                Some(db) => db.trim().parse::<f64>().map(|d| 10f64.powf(d / 20.0)),
                None => t.parse::<f64>(),
            }
            .map_err(|_| format!("filter graph: bad number {s:?}"))
        }
    }
}

/// One filter on the current streams.
fn filter(f: &str, mut cur: Vec<Signal>) -> Result<Vec<Signal>, String> {
    let (name, args) = f.split_once('=').unwrap_or((f, ""));
    let (pos, named) = options(args);
    let arg = |key: &str, i: usize| named.get(key).or(pos.get(i));
    let one = |cur: &mut Vec<Signal>| cur.pop().ok_or_else(|| format!("filter graph: {name} has no input"));
    Ok(match name {
        "asplit" => {
            let n = num(arg("outputs", 0), 2.0)? as usize;
            let s = one(&mut cur)?;
            vec![s; n.max(1)]
        }
        "amix" => {
            let len = cur.iter().map(|s| s.samples.len()).max().unwrap_or(0);
            let rate = cur.first().map_or(RATE, |s| s.rate);
            let mut out = vec![0f32; len];
            for s in &cur {
                let s = if s.rate == rate { s.clone() } else { resample(s, rate) };
                for (o, x) in out.iter_mut().zip(&s.samples) {
                    *o += x;
                }
            }
            vec![Signal { rate, samples: out }]
        }
        _ => {
            let mut s = one(&mut cur)?;
            match name {
                "asetrate" => s.rate = num(arg("sample_rate", 0).or(named.get("r")), s.rate as f64)?.round() as u32,
                "aresample" => s = resample(&s, num(arg("osr", 0), s.rate as f64)?.round() as u32),
                "asetpts" | "anull" => {}
                "volume" => {
                    let g = num(arg("volume", 0), 1.0)? as f32;
                    s.samples.iter_mut().for_each(|x| *x *= g);
                }
                "atrim" => {
                    let start = num(arg("start", 0), 0.0)?;
                    let end = match (arg("end", 1), named.get("duration")) {
                        (Some(e), _) => num(Some(e), 0.0)?,
                        (None, Some(d)) => start + num(Some(d), 0.0)?,
                        (None, None) => f64::INFINITY,
                    };
                    let a = ((start * s.rate as f64).round() as usize).min(s.samples.len());
                    let b = if end.is_finite() { ((end * s.rate as f64).round() as usize).clamp(a, s.samples.len()) } else { s.samples.len() };
                    s.samples = s.samples[a..b].to_vec();
                }
                "adelay" => {
                    // Milliseconds (`17`, `17|17`), or samples with an `S` suffix.
                    let d = arg("delays", 0).map(|v| v.split('|').next().unwrap_or("0").to_string()).unwrap_or_default();
                    let n = match d.strip_suffix('S') {
                        Some(n) => n.parse::<f64>().unwrap_or(0.0) as usize,
                        None => (d.trim_end_matches("ms").parse::<f64>().unwrap_or(0.0) / 1000.0 * s.rate as f64).round() as usize,
                    };
                    let mut v = vec![0.0; n];
                    v.extend_from_slice(&s.samples);
                    s.samples = v;
                }
                "afade" => {
                    let out = arg("type", 0).or(named.get("t")).is_some_and(|t| t == "out");
                    let st = num(named.get("st").or(named.get("start_time")), 0.0)?;
                    let d = num(named.get("d").or(named.get("duration")), 1.0)?.max(1e-6);
                    for (i, x) in s.samples.iter_mut().enumerate() {
                        let t = i as f64 / s.rate as f64;
                        let k = ((t - st) / d).clamp(0.0, 1.0);
                        *x *= if out { 1.0 - k } else { k } as f32;
                    }
                }
                "highpass" | "lowpass" => {
                    let f = num(named.get("f").or(named.get("frequency")).or(pos.first()), if name == "highpass" { 3000.0 } else { 500.0 })?;
                    let q = Biquad::pass(name == "highpass", f, std::f64::consts::FRAC_1_SQRT_2, s.rate as f64);
                    biquad(&mut s, &q);
                }
                "bass" => {
                    let g = named.get("g").or(named.get("gain")).map(|v| v.parse::<f64>().unwrap_or(0.0)).unwrap_or(0.0);
                    let f = num(named.get("f").or(named.get("frequency")), 100.0)?;
                    let q = Biquad::low_shelf(f, g, 0.5, s.rate as f64);
                    biquad(&mut s, &q);
                }
                other => return Err(format!("filter graph: the built-in mixer has no `{other}`")),
            }
            cur.push(s);
            cur
        }
    })
}

/// Linear-interpolation resampling to `rate`.
pub fn resample(s: &Signal, rate: u32) -> Signal {
    if s.rate == rate || s.samples.is_empty() {
        return Signal { rate, samples: s.samples.clone() };
    }
    let ratio = s.rate as f64 / rate as f64;
    let n = ((s.samples.len() as f64) / ratio).round() as usize;
    let last = s.samples.len() - 1;
    let samples = (0..n)
        .map(|i| {
            let x = i as f64 * ratio;
            let j = (x.floor() as usize).min(last);
            let k = (j + 1).min(last);
            let t = (x - j as f64) as f32;
            s.samples[j] * (1.0 - t) + s.samples[k] * t
        })
        .collect();
    Signal { rate, samples }
}

/// A biquad's normalized coefficients (RBJ cookbook).
#[derive(Debug, Clone, Copy)]
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
}

impl Biquad {
    fn norm(b: [f64; 3], a: [f64; 3]) -> Biquad {
        Biquad { b: [b[0] / a[0], b[1] / a[0], b[2] / a[0]], a: [a[1] / a[0], a[2] / a[0]] }
    }

    fn pass(high: bool, f: f64, q: f64, rate: f64) -> Biquad {
        let w = 2.0 * PI * (f / rate).min(0.49);
        let (c, alpha) = (w.cos(), w.sin() / (2.0 * q));
        let b = if high { [(1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0] } else { [(1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0] };
        Biquad::norm(b, [1.0 + alpha, -2.0 * c, 1.0 - alpha])
    }

    /// Low shelf of `gain_db` at `f`, shelf slope `slope`.
    fn low_shelf(f: f64, gain_db: f64, slope: f64, rate: f64) -> Biquad {
        let a = 10f64.powf(gain_db / 40.0);
        let w = 2.0 * PI * (f / rate).min(0.49);
        let (c, sn) = (w.cos(), w.sin());
        let alpha = sn / 2.0 * ((a + 1.0 / a) * (1.0 / slope - 1.0) + 2.0).max(0.0).sqrt();
        let sa = 2.0 * a.sqrt() * alpha;
        Biquad::norm(
            [a * ((a + 1.0) - (a - 1.0) * c + sa), 2.0 * a * ((a - 1.0) - (a + 1.0) * c), a * ((a + 1.0) - (a - 1.0) * c - sa)],
            [(a + 1.0) + (a - 1.0) * c + sa, -2.0 * ((a - 1.0) + (a + 1.0) * c), (a + 1.0) + (a - 1.0) * c - sa],
        )
    }

    /// The two stages of BS.1770's K-weighting at `rate` (libebur128's
    /// derivation: exact at 48 kHz, the same curve at other rates).
    fn k_weighting(rate: f64) -> [Biquad; 2] {
        let (f0, g, q) = (1681.974450955533, 3.999843853973347, 0.7071752369554196);
        let k = (PI * f0 / rate).tan();
        let vh = 10f64.powf(g / 20.0);
        let vb = vh.powf(0.4996667741545416);
        let a0 = 1.0 + k / q + k * k;
        let shelf = Biquad { b: [(vh + vb * k / q + k * k) / a0, 2.0 * (k * k - vh) / a0, (vh - vb * k / q + k * k) / a0], a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0] };
        let (f0, q) = (38.13547087613982, 0.5003270373253953);
        let k = (PI * f0 / rate).tan();
        let a0 = 1.0 + k / q + k * k;
        let high = Biquad { b: [1.0, -2.0, 1.0], a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0] };
        [shelf, high]
    }
}

fn biquad(s: &mut Signal, q: &Biquad) {
    let (mut x1, mut x2, mut y1, mut y2) = (0.0f64, 0.0, 0.0, 0.0);
    for v in &mut s.samples {
        let x = *v as f64;
        let y = q.b[0] * x + q.b[1] * x1 + q.b[2] * x2 - q.a[0] * y1 - q.a[1] * y2;
        x2 = x1;
        x1 = x;
        y2 = y1;
        y1 = y;
        *v = y as f32;
    }
}

/// Integrated loudness (ITU-R BS.1770 / EBU R128, mono) in LUFS: K-weighted
/// 400 ms blocks with 75 % overlap, the absolute (-70) and relative (-10)
/// gates. A sound shorter than one block is measured as one block.
pub fn loudness(s: &Signal) -> f64 {
    let mut k = s.clone();
    let rate = s.rate as f64;
    for q in Biquad::k_weighting(rate) {
        biquad(&mut k, &q);
    }
    let block = (0.4 * rate).round() as usize;
    let hop = (block / 4).max(1);
    let z: Vec<f64> = if k.samples.len() <= block {
        vec![k.samples.iter().map(|&x| (x as f64).powi(2)).sum::<f64>() / block as f64]
    } else {
        (0..=(k.samples.len() - block) / hop).map(|i| k.samples[i * hop..i * hop + block].iter().map(|&x| (x as f64).powi(2)).sum::<f64>() / block as f64).collect()
    };
    let lufs = |z: f64| -0.691 + 10.0 * z.max(1e-20).log10();
    let mean = |v: &[f64]| if v.is_empty() { 0.0 } else { v.iter().sum::<f64>() / v.len() as f64 };
    let abs: Vec<f64> = z.iter().copied().filter(|&z| lufs(z) > -70.0).collect();
    let rel = lufs(mean(&abs)) - 10.0;
    let gated: Vec<f64> = abs.into_iter().filter(|&z| lufs(z) > rel).collect();
    lufs(mean(&gated))
}

/// Sample peak in dBFS.
pub fn peak_db(s: &Signal) -> f64 {
    let p = s.samples.iter().fold(0f32, |m, x| m.max(x.abs())) as f64;
    20.0 * p.max(1e-9).log10()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(f: f64, secs: f64, amp: f32, rate: u32) -> Signal {
        Signal { rate, samples: (0..(secs * rate as f64) as usize).map(|i| amp * (2.0 * PI * f * i as f64 / rate as f64).sin() as f32).collect() }
    }

    #[test]
    fn graph_labels_and_lengths() {
        let a = tone(440.0, 1.0, 0.5, 22050);
        let b = tone(220.0, 0.5, 0.5, 44100);
        // asetrate doubles the rate (half the length), aresample keeps it.
        let out = run_graph(&[a.clone(), b], "[0:a]asetrate=44100,aresample=44100[v];[1:a]volume=0.5[x];[v][x]amix=inputs=2:duration=longest:normalize=0[out]").unwrap();
        assert_eq!(out.rate, 44100);
        assert_eq!(out.samples.len(), 22050);
        // atrim, adelay (ms), asplit.
        let t = run_graph(std::slice::from_ref(&a), "[0:a]atrim=0:0.5,asetpts=PTS-STARTPTS,asplit=2[p][q];[q]adelay=100[q1];[p][q1]amix=inputs=2:normalize=0[out]").unwrap();
        assert_eq!(t.samples.len(), 11025 + 2205);
        assert!(run_graph(&[a], "[0:a]chorus=1[out]").unwrap_err().contains("chorus"));
    }

    #[test]
    fn filters_shape_the_spectrum() {
        let rms = |s: &Signal| (s.samples.iter().map(|x| (*x as f64).powi(2)).sum::<f64>() / s.samples.len() as f64).sqrt();
        let low = tone(60.0, 1.0, 0.5, 44100);
        let high = tone(5000.0, 1.0, 0.5, 44100);
        let hp = run_graph(std::slice::from_ref(&low), "[0:a]highpass=f=450[out]").unwrap();
        assert!(rms(&hp) < rms(&low) * 0.1);
        let lp = run_graph(std::slice::from_ref(&high), "[0:a]lowpass=f=380[out]").unwrap();
        assert!(rms(&lp) < rms(&high) * 0.1);
        let bass = run_graph(std::slice::from_ref(&low), "[0:a]bass=g=6:f=110[out]").unwrap();
        assert!(rms(&bass) > rms(&low) * 1.4);
        let faded = run_graph(&[low], "[0:a]afade=t=out:st=0.5:d=0.25[out]").unwrap();
        assert!(faded.samples[(0.8 * 44100.0) as usize..].iter().all(|x| *x == 0.0));
    }

    #[test]
    fn levels() {
        // A full-scale 1 kHz sine reads -3.01 LUFS (BS.1770's reference).
        let l = loudness(&tone(1000.0, 2.0, 1.0, 48000));
        assert!((l + 3.01).abs() < 0.1, "{l}");
        let wav = render(&[super::wav(&tone(1000.0, 1.0, 0.25, 44100))], "[0:a]volume=1[out]", Level::Peak(-6.0)).unwrap();
        let s = decode(&wav).unwrap();
        assert!((peak_db(&s) + 6.0).abs() < 0.05);
        let wav = render(&[super::wav(&tone(500.0, 1.5, 0.1, 44100))], "[0:a]anull[out]", Level::Lufs(-13.0)).unwrap();
        assert!((loudness(&decode(&wav).unwrap()) + 13.0).abs() < 0.2);
    }
}
