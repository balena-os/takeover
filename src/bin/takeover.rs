use log::error;
use std::process::exit;

use libc;

use mod_logger::Logger;

use takeover::{init, stage1, stage2, ErrorKind, Options};

fn is_init() -> bool {
    let pid = unsafe { libc::getpid() };
    pid == 1
}

#[paw::main]
fn main(opts: Options) {
    let mut exit_code = 0;

    if opts.is_stage2() {
        stage2(&opts);
    } else if is_init() {
        // must only be called if pid is 1 == init
        init(&opts);
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
