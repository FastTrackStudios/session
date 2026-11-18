//! `fts.session` over vox, behind Architect's permissions gate: the caller
//! is the call's principal, and what it may do is the engine's answer. A
//! read-only share link's guest goes through the same check.

use std::sync::Arc;

use architect::layer::{LayerRouter, handler_acceptor};
use architect::permissions_gate::PermissionsGate;
use architect_permissions::{
    Action, Decision, PermissionEngine, Principal, Resource, RoleEngine, StaticPrincipal,
};
use session::doc::SessionDoc;
use session::host::{
    PERMITS, SessionHost, SessionServiceClient, serve, session_service_rpc_service_descriptor,
};
use session::service::{Request, Response};
use session::{MarkId, Region, Time, Value};

const DOC: &str = "song-1";

/// A client of `host`, whose every call the gate resolves to `who` and
/// checks against `engine`: one connection, as one lane sees one caller.
async fn client(
    host: &SessionHost,
    engine: Arc<dyn PermissionEngine>,
    who: Principal,
) -> SessionServiceClient {
    let gate = PermissionsGate::new(engine, Arc::new(StaticPrincipal(who)))
        .permit(session_service_rpc_service_descriptor(), &PERMITS);
    let router = LayerRouter::new()
        .with(
            session_service_rpc_service_descriptor(),
            serve(host.clone()),
        )
        .with_permissions(gate);
    let (client_link, server_link) = vox::memory_link_pair(16);
    tokio::spawn(async move {
        if let Ok(connection) = vox::acceptor_on(server_link)
            .on_lane(handler_acceptor(router))
            .establish_connection()
            .await
        {
            let _hold = connection;
            std::future::pending::<()>().await;
        }
    });
    vox::initiator_on(client_link)
        .establish::<SessionServiceClient>()
        .await
        .unwrap()
}

/// The org's one engine, as Architect's spaces engine is: members by
/// role, and each share link's guest by what the link grants (the exact
/// document with the link's actions, and the gate's pre-decode `doc/**`
/// check by overlap, since the link reaches something under it). An
/// extension is answered for the person it acts for.
struct Org {
    roles: RoleEngine,
    /// Link id → (the document resource, the actions it grants).
    links: Vec<(&'static str, String, &'static [&'static str])>,
}

impl PermissionEngine for Org {
    fn check(&self, who: &Principal, what: &Resource, action: &Action) -> Decision {
        let person = match who {
            Principal::Extension {
                on_behalf_of: Some(person),
                ..
            } => person.as_ref(),
            other => other,
        };
        let Principal::Guest { link_id, .. } = person else {
            return self.roles.check(person, what, action);
        };
        let granted = self.links.iter().any(|(id, doc, actions)| {
            *id == link_id.as_str()
                && (what.0 == *doc || what.0 == "doc/**")
                && actions.contains(&action.0.as_str())
        });
        if granted {
            Decision::Allow
        } else {
            Decision::Deny {
                reason: format!("link {link_id} does not grant {} on {}", action.0, what.0),
            }
        }
    }
}

fn extension(id: &str, person: Principal) -> Principal {
    Principal::Extension {
        id: id.into(),
        on_behalf_of: Some(Box::new(person)),
    }
}

fn alice() -> Principal {
    Principal::User {
        user_id: "alice".into(),
    }
}

/// Alice is a member, Owen an owner; link `link-1` shows song 1, read-only.
fn members() -> Arc<dyn PermissionEngine> {
    let mut roles = RoleEngine::new();
    roles.set_member("alice", "member");
    roles.set_member("owen", "owner");
    Arc::new(Org {
        roles,
        links: vec![("link-1", format!("doc/{DOC}"), &["read"])],
    })
}

fn host_with_session() -> (SessionHost, Arc<dyn PermissionEngine>) {
    let engine = members();
    let host = SessionHost::new(engine.clone());
    host.open(DOC, SessionDoc::new());
    (host, engine)
}

fn splice(text: &str) -> Request {
    Request::TextSplice {
        name: "chart".into(),
        at: 0,
        delete: 0,
        insert: text.into(),
    }
}

fn refused<T: std::fmt::Debug>(r: Result<T, impl std::fmt::Debug>) -> String {
    format!("{:?}", r.unwrap_err())
}

#[tokio::test]
async fn an_extension_writes_its_own_namespace_as_its_principal() {
    let (host, engine) = host_with_session();
    let keyflow = client(&host, engine.clone(), extension("keyflow", alice())).await;
    let ignition = client(&host, engine, extension("ignition", alice())).await;

    keyflow
        .ask(DOC.into(), splice("Verse: C G\n"))
        .await
        .unwrap();
    ignition
        .ask(
            DOC.into(),
            Request::Set {
                name: "rig".into(),
                value: Value::from("Club"),
            },
        )
        .await
        .unwrap();

    let Response::Data(k) = keyflow.ask(DOC.into(), Request::ReadData).await.unwrap() else {
        panic!("not data")
    };
    assert_eq!(k.text("chart"), Some("Verse: C G\n"));
    assert_eq!(
        k.register("rig"),
        None,
        "Keyflow sees only its own namespace"
    );
    let Response::Data(i) = ignition.ask(DOC.into(), Request::ReadData).await.unwrap() else {
        panic!("not data")
    };
    assert_eq!(i.register("rig"), Some(&Value::from("Club")));
    assert_eq!(i.text("chart"), None);
}

#[tokio::test]
async fn a_read_only_link_guest_reads_and_cannot_write() {
    let (host, engine) = host_with_session();
    // Alice writes the chart first, as a member.
    let author = client(&host, engine.clone(), extension("keyflow", alice())).await;
    author
        .ask(DOC.into(), splice("Verse: C G\n"))
        .await
        .unwrap();

    // A guest of a read-only link to this session: the same engine, the
    // same handler, only the principal differs.
    let guest = Principal::Guest {
        link_id: "link-1".into(),
        display: Some("Fan".into()),
    };
    let viewer = client(&host, engine, extension("keyflow", guest)).await;

    let Response::Data(k) = viewer.ask(DOC.into(), Request::ReadData).await.unwrap() else {
        panic!("not data")
    };
    assert_eq!(k.text("chart"), Some("Verse: C G\n"));
    viewer.ask(DOC.into(), Request::ReadTimeline).await.unwrap();
    let denied = refused(viewer.ask(DOC.into(), splice("vandal ")).await);
    assert!(denied.contains("may not write"), "{denied}");
    // Nothing was written.
    let Response::Data(k) = author.ask(DOC.into(), Request::ReadData).await.unwrap() else {
        panic!("not data")
    };
    assert_eq!(k.text("chart"), Some("Verse: C G\n"));

    // A session the link does not cover is refused by the handler's exact
    // check, before it is looked up.
    host.open("song-2", SessionDoc::new());
    let other = refused(viewer.ask("song-2".into(), Request::ReadData).await);
    assert!(other.contains("may not read session song-2"), "{other}");
}

#[tokio::test]
async fn only_an_extension_asks_and_only_open_sessions_answer() {
    let (host, engine) = host_with_session();
    let person = client(&host, engine.clone(), alice()).await;
    let denied = refused(person.ask(DOC.into(), Request::ReadData).await);
    assert!(denied.contains("only an extension"), "{denied}");

    let keyflow = client(&host, engine, extension("keyflow", alice())).await;
    let missing = keyflow
        .ask("song-9".into(), Request::ReadData)
        .await
        .unwrap_err();
    assert!(format!("{missing:?}").contains("NotFound"), "{missing:?}");
}

#[tokio::test]
async fn the_song_structure_needs_the_structure_action() {
    let (host, engine) = host_with_session();
    let verse = || Request::PutRegion {
        id: MarkId::from("verse"),
        region: Region {
            start: Time::QuarterNotes(0.0),
            end: Time::QuarterNotes(16.0),
            name: "Verse".into(),
            color: None,
            ruler: None,
        },
    };
    // A member reads and writes, but `structure` is no member's action.
    let member = client(&host, engine.clone(), extension("keyflow", alice())).await;
    let denied = refused(member.ask(DOC.into(), verse()).await);
    assert!(denied.contains("may not structure"), "{denied}");

    // An owner holds every action.
    let owen = Principal::User {
        user_id: "owen".into(),
    };
    let owner = client(&host, engine, extension("keyflow", owen)).await;
    owner.ask(DOC.into(), verse()).await.unwrap();
}
