#[macro_export]
macro_rules! upstream_context {
    (  $x:expr  ) => {{
        MigErrCtx::from_remark(ErrorKind::Upstream, $x)
    }};
}

macro_rules! call_command {
    (  $cmd:expr, $args:expr , $errmsg:expr ) => {
        match call($cmd, $args, true) {
            Ok(cmd_res) => {
                if cmd_res.status.success() {
                    Ok(cmd_res.stdout)
                } else {
                    Err(Error::with_context(
                        ErrorKind::ExecProcess,
                        &format!("{}, stderr: {}", $errmsg, cmd_res.stderr),
                    ))
                }
            }
            Err(why) => Err(why),
        }
    };
    (  $cmd:expr, $args:expr  ) => {
        match call($cmd, $args, true) {
            Ok(cmd_res) => {
                if cmd_res.status.success() {
                    Ok(cmd_res.stdout)
                } else {
                    Err(Error::with_context(
                        ErrorKind::ExecProcess,
                        &format!("stderr: {}", cmd_res.stderr),
                    ))
                }
            }
            Err(why) => Err(why),
        }
    };
}

macro_rules! call_busybox {
    (  $args:expr , $errmsg:expr ) => {
        call_command!(BUSYBOX_CMD, $args, $errmsg)
    };
    (  $args:expr ) => {
        call_command!(BUSYBOX_CMD, $args)
    };
}
