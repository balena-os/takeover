#[macro_export]
macro_rules! upstream_context {
    (  $x:expr  ) => {{
        MigErrCtx::from_remark(MigErrorKind::Upstream, $x)
    }};
}

macro_rules! from_upstream {
    (  $err:expr, $comment:expr  ) => {{
        MigError::from($err.context(upstream_context!($comment)))
    }};
}

macro_rules! call_command {
    (  $cmd:expr, $args:expr , $errmsg:expr ) => {
        match call($cmd, $args, true) {
            Ok(cmd_res) => {
                if cmd_res.status.success() {
                    Ok(cmd_res.stdout)
                } else {
                    Err(MigError::from_remark(
                        MigErrorKind::ExecProcess,
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
                    Err(MigError::from_remark(
                        MigErrorKind::ExecProcess,
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
