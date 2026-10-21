# Provenance: sound/: sound effects, voice, UI sounds, music

Where each file of this directory comes from (paths relative to it).
Licence, the upscaling and the layout: `docs/layout.md` (§ Provenance).

This table lists OpenTT's own files of this directory; the importer builds the rest of the asset directory from your game install (`docs/import.md`).

| file | source: how |
|---|---|
| `music/spacefighter.ogg` | free licence: "Space Fighter Loop" Kevin MacLeod (incompetech.com), Licensed under Creative Commons: By Attribution 4.0 License, http://creativecommons.org/licenses/by/4.0/ ; downloaded from https://incompetech.com/music/royalty-free/index.html?isrc=USUAN1100672 (320 kb/s MP3), cut with ffmpeg 4.4 to 20.215..101.07 s (the quiet intro dropped, 32 bars that loop), +2 dB with a limiter, 4 ms fade-in and 30 ms fade-out at the loop seam, Ogg Vorbis q5 (the Moon maps' music, MusicDef "Lunar") |
| `voice/vocarea.ogg` | generated: Piper TTS voice en_US-norman-medium (local), light tanh overdrive + compression |
| `voice/vocbounce.ogg` | generated: Piper TTS voice en_US-norman-medium (local), light tanh overdrive + compression |
| `voice/vochealth.ogg` | generated: Piper TTS voice en_US-norman-medium (local), light tanh overdrive + compression |
| `voice/vocmortar.ogg` | generated: Piper TTS voice en_US-norman-medium (local), light overdrive, loudness matched to the other announcer lines ("Mortar powerup!", length scale 0.9; OpenTT's own power-up) |
| `voice/vocreload.ogg` | generated: Piper TTS voice en_US-norman-medium (local), light tanh overdrive + compression |
| `voice/vocscatter.ogg` | generated: Piper TTS voice en_US-norman-medium (local), light overdrive, loudness matched to the other announcer lines ("Scattershot power-up!", length scale 0.9; OpenTT's own power-up) |
| `voice/vocspeedy.ogg` | generated: Piper TTS voice en_US-norman-medium (local), light tanh overdrive + compression |
