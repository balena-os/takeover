use log::error;
use std::path::PathBuf;
use std::process::exit;

use mod_logger::{Level, LogDestination, Logger, NO_STREAM};

use takeover::{stage1, stage2, MigErrorKind, Options};

#[paw::main]
fn main(opts: Options) {
    Logger::set_default_level(Level::Info);
    Logger::set_brief_info(true);
    Logger::set_color(true);

    if opts.is_stage2() {
        if let Err(why) = Logger::set_log_dest(&LogDestination::BufferStderr, NO_STREAM) {
            error!("Failed to initialize logging, error: {:?}", why);
            exit(1);
        }

        if let Err(why) = stage2(opts) {
            match why.kind() {
                MigErrorKind::Displayed => (),
                _ => error!("Migrate stage 2 returned error: {:?}", why),
            };
            Logger::flush();
            exit(1);
        };
    } else {
        let log_file = PathBuf::from("./stage1.log");
        if let Err(why) = Logger::set_log_file(&LogDestination::StreamStderr, &log_file, true) {
            error!(
                "Failed to set logging to '{}', error: {:?}",
                log_file.display(),
                why
            );
            exit(1);
        }

        if let Err(why) = stage1(opts) {
            match why.kind() {
                MigErrorKind::Displayed => (),
                _ => error!("Migrate stage 1 returned error: {:?}", why),
            };
            Logger::flush();
            exit(1);
        };
    }

    Logger::flush();
}
