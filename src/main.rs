#[macro_use]
mod macros;
mod common;
mod init;
mod stage1;
mod stage2;

use log::error;
use std::process::exit;

use clap::Parser;
use mod_logger::Logger;

use crate::{
    common::{error::ErrorKind, Options},
    init::init,
    stage1::stage1,
    stage2::stage2,
};

fn is_init() -> bool {
    let pid = unsafe { libc::getpid() };
    pid == 1
}

fn main() {
    let mut exit_code = 0;

    if is_init() {
        init();
    } else {
        let opts = Options::parse();

        if opts.stage2() {
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
