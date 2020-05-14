#[macro_export]
macro_rules! upstream_context {
    (  $x:expr  ) => {{
        MigErrCtx::from_remark(MigErrorKind::Upstream, $x)
    }};
}
