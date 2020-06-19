use log::error;
use std::process::exit;

use mod_logger::Logger;

use takeover::{init, stage1, stage2, MigErrorKind, Options};

#[paw::main]
fn main(opts: Options) {
    let mut exit_code = 0;

    if opts.is_stage2() {
        stage2(&opts);
        // not supposed to return
        exit_code = 1;
    } else if opts.is_init() {
        init(&opts);
        // not supposed to return
        exit_code = 1;
    } else if let Err(why) = stage1(&opts) {
        exit_code = 1;
        match why.kind() {
            MigErrorKind::Displayed => (),
            _ => error!("Migrate stage 1 returned error: {:?}", why),
        };
    };

    Logger::flush();
    exit(exit_code);
}
