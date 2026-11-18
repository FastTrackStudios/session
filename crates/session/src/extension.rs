//! `fts.session` as an Architect extension service (feature `extension`).
//!
//! Inside an Architect app, one extension asks another through the host:
//! the caller declares `[requires] "fts.session"`, calls
//! `services.call("fts.session", method, args)` (`HostRequest::Service`),
//! and the host runs the provider's action `<PREFIX><METHOD>` **as the
//! caller**, with the caller's id in the action's context (`cx.caller`,
//! from the call's principal: the host's word, never the arguments).
//! [`answer`] is what that action does. It needs no Architect crate: the
//! provider registers three actions that hand it their JSON.
//!
//! **Methods.** One per action a request needs ([`crate::service::action_for`]),
//! so Architect's own effect check holds them:
//!
//! | method      | action                  | effect      | requests |
//! |-------------|-------------------------|-------------|----------|
//! | `read`      | `<PREFIX>READ`          | `read_only` | `ReadData`, `ReadTimeline` |
//! | `write`     | `<PREFIX>WRITE`         | `mutating`  | the caller's namespace, lanes and items |
//! | `structure` | `<PREFIX>STRUCTURE`     | `mutating`  | tempo, markers, regions |
//!
//! A share link's guest runs its extensions `read_only`, and the host
//! refuses a service call above the caller's effect before the provider
//! runs: a read-only link's chart view reads, and its edits never arrive.
//! That is Architect's permission, not a path here. [`answer`] refuses a
//! request sent through the wrong method, so the effect a method declares
//! is the effect it has. Who may change the song's structure is who the
//! host lets run `structure`.
//!
//! **The provider's manifest** (its prefix is its own, here `SESSION_`):
//!
//! ```toml
//! [provides]
//! "fts.session" = "0.1.0"
//!
//! [[actions]]
//! id = "SESSION_READ"
//! effect = "read_only"
//!
//! [[actions]]
//! id = "SESSION_WRITE"
//! effect = "mutating"
//!
//! [[actions]]
//! id = "SESSION_STRUCTURE"
//! effect = "mutating"
//! ```
//!
//! and its code, for each: `.action_in("SESSION_READ", |args, cx| async
//! move { answer(&sessions, READ, &args.to_string(), cx["caller"].as_str()) })`,
//! parsing the reply back to JSON.
//!
//! **Arguments:** `{"doc": "<session id>", "request": <Request>}`; the
//! reply is a [`Response`]. Both are this crate's types in `facet-json`'s
//! encoding (an enum externally tagged: `{"TextSplice": {...}}`).

use crate::id::ExtensionId;
use crate::service::{
    Grants, Request, Response, STRUCTURE, ServiceError, Sessions, action_for, handle,
};

/// The service's name.
pub const SERVICE: &str = "fts.session";

/// A call's arguments.
#[derive(Debug, Clone, PartialEq, facet::Facet)]
pub struct Call {
    /// The session's document id.
    pub doc: String,
    pub request: Request,
}

/// Answers one call to the service, as the provider's action does.
///
/// `method` is `read`, `write` or `structure`; `args` a [`Call`] as JSON;
/// `caller` the extension in the action context's `caller`. It answers
/// against the sessions open in `sessions`, with a [`Response`] as JSON.
///
/// # Errors
///
/// A message when there is no caller (only an extension asks), the
/// arguments do not parse, the request does not belong to the method,
/// the session is not open, or [`handle`] refused or failed.
pub fn answer(
    sessions: &Sessions,
    method: &str,
    args: &str,
    caller: Option<&str>,
) -> Result<String, String> {
    let caller = caller
        .filter(|c| !c.is_empty())
        .ok_or("only an extension asks the session (no caller in the context)")?;
    let call: Call = facet_json::from_str(args).map_err(|e| format!("arguments: {e}"))?;
    let needs = action_for(&call.request);
    if needs != method {
        return Err(format!(
            "this request is a `{needs}`, not a `{method}`: call method `{needs}`"
        ));
    }
    let session = sessions
        .get(&call.doc)
        .ok_or_else(|| format!("no session {} is open here", call.doc))?;
    let grants = Grants {
        structure: method == STRUCTURE,
    };
    let response = handle(&session, &ExtensionId::from(caller), grants, call.request).map_err(
        |e| match e {
            ServiceError::Refused(why) => format!("refused: {why}"),
            ServiceError::Failed(e) => format!("failed: {e}"),
        },
    )?;
    facet_json::to_string(&response).map_err(|e| format!("reply: {e}"))
}

/// A call's arguments as JSON: what a caller passes `services.call`.
///
/// # Errors
///
/// The request could not be encoded.
pub fn call_json(doc: &str, request: Request) -> Result<String, String> {
    facet_json::to_string(&Call {
        doc: doc.to_owned(),
        request,
    })
    .map_err(|e| e.to_string())
}

/// A reply as a [`Response`].
///
/// # Errors
///
/// The JSON is not a response.
pub fn response_of(json: &str) -> Result<Response, String> {
    facet_json::from_str(json).map_err(|e| e.to_string())
}
