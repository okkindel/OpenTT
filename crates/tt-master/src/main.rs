//! `tt-master [--port N] [--status SECONDS] [--local-address IP] [--stats-port N [--stats-user NAME]]`:
//! the master server (`/stats` password in `OPENTT_MASTER_STATS_PASSWORD`).

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let o = match tt_master::parse_args(&args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    if let Err(e) = tt_master::run(&o) {
        eprintln!("tt-master: {e}");
        std::process::exit(1);
    }
}
