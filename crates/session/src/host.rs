//! The `fts.session` service: [`crate::service::handle`] over vox (feature
//! `vox`).
//!
//! A host that holds sessions serves [`SessionHost`]; an extension's
//! client calls [`SessionService::ask`] with the session's document id
//! and a [`Request`].
//!
//! - **Who is asking is the call's principal**, as the permissions gate
//!   resolved it (`architect::permissions_gate::caller`), never an
//!   argument. An extension calls as `Principal::Extension { id,
//!   on_behalf_of }`, and `id` is its namespace. Any other principal is
//!   refused: a person's own view holds the document, it does not ask.
//! - **What it may do is the permission engine's answer** for that
//!   principal, on the document's resource (`doc/<id>`, org-relative, as
//!   Architect's documents are): `read` for [`Request::ReadData`] and
//!   [`Request::ReadTimeline`], `write` for every edit, and `structure` for
//!   the tempo map, markers and regions. A share link's guest goes through
//!   the same check: a read-only link's scope grants `read` and nothing
//!   else, so its extension reads and every edit is refused. There is no
//!   guest code path.
//! - The gate checks `read` on `doc/{doc}` before the handler runs
//!   ([`PERMITS`]); the handler checks the exact action.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use architect_permissions::{Action, Decision, PermissionEngine, Principal, Resource};

use crate::doc::SessionDoc;
use crate::id::ExtensionId;
use crate::service::{Grants, Request, Response, ServiceError, handle};

/// Why a call was not answered.
#[architect::error]
#[derive(Eq)]
pub enum SessionError {
    /// Outside what the caller may do.
    #[error("refused: {0}")]
    #[architect(http_status = 403)]
    Refused(String),
    /// No session with that id is open here.
    #[error("no session {0}")]
    #[architect(http_status = 404)]
    NotFound(String),
    /// Allowed, and the document refused the write.
    #[error("failed: {0}")]
    Failed(String),
}

/// The service extensions ask the session through.
#[architect::rpc]
pub trait SessionService {
    /// Answers `request` against session `doc` (its document id) for the
    /// calling extension.
    async fn ask(&self, doc: String, request: Request) -> Result<Response, SessionError>;
}

/// The name the service is provided under (`[provides]`).
pub const SERVICE: &str = "fts.session";

/// The gate's table: `ask` needs `read` on the document; the handler
/// checks the exact action.
pub const PERMITS: architect_permissions::ServicePermits = architect_permissions::ServicePermits {
    service: SERVICE,
    methods: &[architect_permissions::MethodPermit::new(
        "ask",
        "read",
        "doc/{doc}",
    )],
};

/// The action a request needs on the document.
#[must_use]
pub const fn action_for(request: &Request) -> &'static str {
    match request {
        Request::ReadData | Request::ReadTimeline => Action::READ,
        Request::SetTempo { .. }
        | Request::PutMarker { .. }
        | Request::PutRegion { .. }
        | Request::RemoveMark { .. } => STRUCTURE,
        Request::Set { .. }
        | Request::MapSet { .. }
        | Request::MapRemove { .. }
        | Request::TextSplice { .. }
        | Request::Remove { .. }
        | Request::PutLane { .. }
        | Request::RemoveLane { .. }
        | Request::PutItem { .. }
        | Request::RemoveItem { .. } => Action::WRITE,
    }
}

/// The action that writes the song's structure (tempo, markers, regions).
pub const STRUCTURE: &str = "structure";

/// The namespace a principal's call writes: the extension's id.
///
/// # Errors
///
/// [`SessionError::Refused`] for any principal that is not an extension.
pub fn extension_of(who: &Principal) -> Result<ExtensionId, SessionError> {
    match who {
        Principal::Extension { id, .. } => Ok(ExtensionId(id.clone())),
        other => Err(SessionError::Refused(format!(
            "only an extension asks the session, not {}",
            other.describe()
        ))),
    }
}

/// The sessions a host serves, by document id.
#[derive(Clone)]
pub struct SessionHost {
    sessions: Arc<RwLock<HashMap<String, SessionDoc>>>,
    engine: Arc<dyn PermissionEngine>,
}

impl SessionHost {
    /// A host that asks `engine` what each caller may do.
    ///
    /// The same engine the permissions gate in front of it asks: the
    /// org's one engine, which answers for members by role and for a share
    /// link's guests by what the link grants (Architect's spaces engine).
    /// A host checking against another engine than its gate would judge
    /// a guest by rules the link never set.
    #[must_use]
    pub fn new(engine: Arc<dyn PermissionEngine>) -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            engine,
        }
    }

    /// Serves `session` under its document id.
    pub fn open(&self, doc: impl Into<String>, session: SessionDoc) {
        architect::write(&self.sessions).insert(doc.into(), session);
    }

    /// Stops serving `doc`.
    pub fn close(&self, doc: &str) {
        architect::write(&self.sessions).remove(doc);
    }

    /// [`SessionService::ask`] for `who`: what the handler does once it
    /// knows the caller. Public so a host that holds the session in
    /// process (a linked view) asks through the same rules.
    ///
    /// # Errors
    ///
    /// [`SessionError::Refused`] when `who` is not an extension or the
    /// engine denies the action; [`SessionError::NotFound`] for a session
    /// not open here; [`SessionError::Failed`] when the write failed.
    pub fn ask_as(
        &self,
        who: &Principal,
        doc: &str,
        request: Request,
    ) -> Result<Response, SessionError> {
        let ext = extension_of(who)?;
        let resource = Resource::new(format!("doc/{doc}"));
        let action = action_for(&request);
        if let Decision::Deny { reason } = self.engine.check(who, &resource, &Action::new(action)) {
            return Err(SessionError::Refused(format!(
                "{} may not {action} session {doc}: {reason}",
                who.describe()
            )));
        }
        let session = architect::read(&self.sessions)
            .get(doc)
            .cloned()
            .ok_or_else(|| SessionError::NotFound(doc.to_owned()))?;
        // The structure action was checked above for the requests that
        // need it; the grant only says so to `handle`.
        let grants = Grants {
            structure: action == STRUCTURE,
        };
        handle(&session, &ext, grants, request).map_err(|e| match e {
            ServiceError::Refused(why) => SessionError::Refused(why),
            ServiceError::Failed(e) => SessionError::Failed(e.to_string()),
        })
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl SessionService for SessionHost {
    // The trait is async (a vox service); this answer needs no await.
    #[allow(clippy::unused_async_trait_impl)]
    async fn ask(&self, doc: String, request: Request) -> Result<Response, SessionError> {
        let who = architect::permissions_gate::caller().ok_or_else(|| {
            SessionError::Refused("no caller: serve it behind the permissions gate".into())
        })?;
        self.ask_as(&who, &doc, request)
    }
}
