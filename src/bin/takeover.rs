use libc;
use log::error;
use std::process::exit;

use mod_logger::Logger;
use structopt::StructOpt;

use takeover::{init, stage1, stage2, ErrorKind, Options};

fn is_init() -> bool {
    let pid = unsafe { libc::getpid() };
    pid == 1
}

fn main() {
    let mut exit_code = 0;

    if is_init() {
        init();
    } else {
        let opts = Options::from_args();

        if opts.is_stage2() {
            stage2(&opts);
        } else if let Err(why) = stage1(&opts) {
            exit_code = 1;
            match why.kind() {
                ErrorKind::Displayed => (),
                _ => error!("Migrate stage 1 returned an error: {}", why),
            };
        };
        Logger::flush();
        exit(exit_code);
    }
}
