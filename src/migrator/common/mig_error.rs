use failure::{Backtrace, Context, Fail};
use std::fmt::{self, Display};

#[derive(Copy, Clone, Eq, PartialEq, Debug, Fail)]
pub enum MigErrorKind {
    #[fail(display = "A required item could not be found")]
    NotFound,
    #[fail(display = "An duplicate item was encountered where it should be unique")]
    Duplicate,
    #[fail(display = "An error occured in an upstream function")]
    Upstream,
    #[fail(display = "An unknown error occurred")]
    Unknown,
    #[fail(display = "The OS type is not supported")]
    InvOSType,
    #[fail(display = "The function has not been implemented yet")]
    NotImpl,
    #[fail(display = "A command IO stream operation failed")]
    CmdIO,
    #[fail(display = "An invalid value was encountered")]
    InvParam,
    #[fail(display = "An invalid state was encountered")]
    InvState,
    #[fail(display = "A required program could not be found")]
    PgmNotFound,
    #[fail(display = "A required feature is not available")]
    FeatureMissing,
    #[fail(display = "A spawned process returned an error code")]
    ExecProcess,
    #[fail(display = "An error occurred calling a WINAPI")]
    WinApi,
    #[fail(display = "Initialization of WMI")]
    WmiInit,
    #[fail(display = "A WMI query failed")]
    WmiQueryFailed,
    #[fail(display = "A Powershell command failed")]
    PSFailed,
    #[fail(display = "You are not authorized to execute this command")]
    AuthError,
    #[fail(display = "Mutual access failed")]
    MutAccess,
    #[fail(display = "No Match")]
    NoMatch,
    #[fail(display = "Timeout waiting for event")]
    Timeout,

    // errors that have had their messages displayed elsewhere
    #[fail(display = "Displayed")]
    Displayed,
}

pub struct MigErrCtx {
    kind: MigErrorKind,
    descr: String,
}

impl MigErrCtx {
    pub fn from_remark(kind: MigErrorKind, descr: &str) -> MigErrCtx {
        MigErrCtx {
            kind,
            descr: String::from(descr),
        }
    }
}

impl From<MigErrorKind> for MigErrCtx {
    fn from(kind: MigErrorKind) -> MigErrCtx {
        MigErrCtx {
            kind,
            descr: String::new(),
        }
    }
}

impl Display for MigErrCtx {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.descr.is_empty() {
            write!(f, "Error: {}", self.kind)
        } else {
            write!(f, "Error: {}, {}", self.kind, self.descr)
        }
    }
}

#[derive(Debug)]
pub struct MigError {
    inner: Context<MigErrCtx>,
}

impl Fail for MigError {
    fn name(&self) -> Option<&str> {
        self.inner.name()
    }

    fn cause(&self) -> Option<&dyn Fail> {
        self.inner.cause()
    }

    fn backtrace(&self) -> Option<&Backtrace> {
        self.inner.backtrace()
    }
}

impl Display for MigError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut res = Display::fmt(&self.inner, f);
        if let Some(fail) = self.inner.cause() {
            write!(f, " - ")?;
            res = Display::fmt(fail, f);
        }
        res
    }
}

impl MigError {
    pub fn kind(&self) -> MigErrorKind {
        self.inner.get_context().kind
    }

    pub fn from_remark(kind: MigErrorKind, remark: &str) -> MigError {
        MigError {
            inner: Context::new(MigErrCtx::from_remark(kind, remark)),
        }
    }

    pub fn displayed() -> MigError {
        MigError::from(MigErrorKind::Displayed)
    }

    /*
        pub fn upstream() -> MigError {

        }
    */
}

impl From<MigErrorKind> for MigError {
    fn from(kind: MigErrorKind) -> MigError {
        MigError {
            inner: Context::new(MigErrCtx::from(kind)),
        }
    }
}

impl From<MigErrCtx> for MigError {
    fn from(mig_ctxt: MigErrCtx) -> MigError {
        MigError {
            inner: Context::new(mig_ctxt),
        }
    }
}

impl From<Context<MigErrCtx>> for MigError {
    fn from(inner: Context<MigErrCtx>) -> MigError {
        MigError { inner }
    }
}
