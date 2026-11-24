use std::ffi::OsString;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use infrarust_api::BoxFuture;

use crate::binary::{
    BuildInfo, ExeSnapshot, HANDOVER_ACCEPTS, HANDOVER_PROTOCOL, HandoverProbe, HandoverSupport,
    PROBE_SCHEMA, RunningBinary,
};
use crate::hot_restart::{Preflight, PreflightError, RestartPlan};

pub fn test_binary(exe: &Path) -> Arc<RunningBinary> {
    RunningBinary::new(
        BuildInfo {
            version: env!("CARGO_PKG_VERSION"),
            target: "test-target",
            official: false,
            docker: false,
        },
        ExeSnapshot::at(exe.to_path_buf()),
        Vec::<OsString>::new(),
    )
}

#[derive(Debug)]
pub struct FakePreflight {
    approves: bool,
    checks: AtomicUsize,
}

impl FakePreflight {
    pub fn approving() -> Arc<Self> {
        Arc::new(Self {
            approves: true,
            checks: AtomicUsize::new(0),
        })
    }

    pub fn refusing() -> Arc<Self> {
        Arc::new(Self {
            approves: false,
            checks: AtomicUsize::new(0),
        })
    }

    pub fn checks(&self) -> usize {
        self.checks.load(Ordering::SeqCst)
    }
}

impl Preflight for FakePreflight {
    fn check<'a>(
        &'a self,
        plan: &'a RestartPlan,
    ) -> BoxFuture<'a, Result<HandoverProbe, PreflightError>> {
        self.checks.fetch_add(1, Ordering::SeqCst);
        let build = plan.binary.build;
        let outcome = if self.approves {
            Ok(HandoverProbe {
                probe: PROBE_SCHEMA,
                version: build.version.to_string(),
                target: build.target.to_string(),
                official: build.official,
                docker: build.docker,
                handover: Some(HandoverSupport {
                    protocol: HANDOVER_PROTOCOL,
                    accepts: HANDOVER_ACCEPTS.to_vec(),
                }),
            })
        } else {
            Err(PreflightError::NoHandover {
                path: plan.target.display().to_string(),
                version: build.version.to_string(),
            })
        };
        Box::pin(async move { outcome })
    }
}
