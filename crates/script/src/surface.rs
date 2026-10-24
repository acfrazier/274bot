//! Sealed external-script input authority and the shared host envelope.
//!
//! This module defines no script loader or authoring binding. The loader's
//! admitted capability metadata remains the only source of an external permit.

use api::interact::server_ordered::ServerOrderedRequest;
use api::prot::Send;
use api::selected::RunKey;
use std::sync::Arc;

use crate::native::{ActionContext, ActionError};

/// One owned request, with no public constructor for advanced packet input.
pub struct ServerOrderedEnvelope {
    payload: Payload,
}

enum Payload {
    Content(ServerOrderedRequest),
    Advanced(AdvancedPacketOps),
}

/// A borrowed payload is dispatch data, not authority to emit another request.
pub enum ServerOrderedPayload<'a> {
    Content(&'a ServerOrderedRequest),
    Advanced(&'a [Send]),
}

impl ServerOrderedEnvelope {
    pub(crate) fn content(request: ServerOrderedRequest) -> Self {
        Self {
            payload: Payload::Content(request),
        }
    }

    pub fn payload(&self) -> ServerOrderedPayload<'_> {
        match &self.payload {
            Payload::Content(request) => ServerOrderedPayload::Content(request),
            Payload::Advanced(request) => ServerOrderedPayload::Advanced(&request.operations),
        }
    }

    pub fn debug_catalog(&self) -> Option<&api::debug_commands::DebugCatalog> {
        match &self.payload {
            Payload::Content(_) => None,
            Payload::Advanced(request) => request.debug.as_deref(),
        }
    }
}

/// An external packet batch can be constructed only within this surface.
///
/// ```compile_fail,E0451
/// let _ = script::surface::AdvancedPacketOps { operations: Vec::new(), debug: None };
/// ```
pub struct AdvancedPacketOps {
    operations: Vec<Send>,
    debug: Option<Arc<api::debug_commands::DebugCatalog>>,
}

/// A permit binds the external entry run and session admitted by its loader.
/// Native machines receive neither this permit nor a packet context.
///
/// ```compile_fail,E0451
/// let _ = script::surface::ScriptPacketOpsPermit { run: todo!(), debug: None };
/// ```
pub struct ScriptPacketOpsPermit {
    run: RunKey,
    debug: Option<Arc<api::debug_commands::DebugCatalog>>,
}

/// The external surface can emit packet input with its sealed live permit.
/// Built-in machines receive only [`ActionContext`].
///
/// ```compile_fail,E0599
/// fn native(cx: &mut script::native::ActionContext<'_>, packets: Vec<api::prot::Send>) {
///     cx.send_batch(packets);
/// }
/// ```
pub struct ScriptPacketOpsContext<'context, 'frame> {
    context: &'context mut ActionContext<'frame>,
    permit: ScriptPacketOpsPermit,
}

impl ScriptPacketOpsContext<'_, '_> {
    /// Queue one bounded packet batch. This does not prove server acceptance.
    pub fn send_batch(&mut self, operations: Vec<Send>) -> Result<u64, ActionError> {
        if self.context.run() != self.permit.run {
            return Err(ActionError::Stale);
        }
        ServerOrderedRequest::validate_advanced_shape(self.context.pin.revision, &operations)
            .map_err(ActionError::ServerOrdered)?;
        self.context
            .emit_server_ordered_envelope(ServerOrderedEnvelope {
                payload: Payload::Advanced(AdvancedPacketOps {
                    operations,
                    debug: self.permit.debug.as_ref().map(Arc::clone),
                }),
            })
    }
}

/// Integration fixtures can exercise the real envelope without a script loader.
/// This constructor is absent from every production feature lane.
#[cfg(any(test, feature = "test-hooks"))]
pub fn packet_context_for_test<'context, 'frame>(
    context: &'context mut ActionContext<'frame>,
) -> ScriptPacketOpsContext<'context, 'frame> {
    let run = context.run();
    ScriptPacketOpsContext {
        context,
        permit: ScriptPacketOpsPermit { run, debug: None },
    }
}

/// Integration fixtures can bind the same optional catalog as an external loader.
#[cfg(any(test, feature = "test-hooks"))]
pub fn packet_context_for_test_with_debug<'context, 'frame>(
    context: &'context mut ActionContext<'frame>,
    debug: Arc<api::debug_commands::DebugCatalog>,
) -> ScriptPacketOpsContext<'context, 'frame> {
    let run = context.run();
    ScriptPacketOpsContext {
        context,
        permit: ScriptPacketOpsPermit {
            run,
            debug: Some(debug),
        },
    }
}
