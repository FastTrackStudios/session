//! The review panel, driven the way a player drives it.
//!
//! The rules about what a mark MEANS are tested in
//! `session_proto::review`, without a DOM. What is tested here is the
//! part only a rendered component can be wrong about: that the four
//! buttons are there, that pressing one reports the verdict it shows,
//! and that a verdict with nothing selected is about the whole take —
//! which is the difference between "that take was good" and "the first
//! nothing of it was good".

use std::sync::{Mutex, OnceLock};

use dioxus::prelude::*;
use dioxus_test::{by_testid, render};
use session_proto::review::{Mark, Pass, Verdict};

/// What the panel reported, in the order it reported it.
fn marked() -> &'static Mutex<Vec<Mark>> {
    static MARKED: OnceLock<Mutex<Vec<Mark>>> = OnceLock::new();
    MARKED.get_or_init(|| Mutex::new(Vec::new()))
}

/// Who the tablet belongs to, once somebody says.
fn performer() -> &'static Mutex<Option<String>> {
    static WHO: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    WHO.get_or_init(|| Mutex::new(None))
}

const LENGTH: f64 = 182.0;

#[component]
fn Harness() -> Element {
    let mut who = use_signal(|| performer().lock().expect("who").clone());
    let mut pass = use_signal(|| {
        let mut pass = Pass::new(4, 0.0, LENGTH);
        pass.mark(Mark::whole("Joshua", LENGTH, Verdict::VeryGood));
        pass
    });

    rsx! {
        // What the test reads back: the panel's own state is inside
        // Dioxus, and a test outside the runtime can only see the DOM.
        div {
            "data-testid": "debug",
            "data-marks": "{pass().marks.len()}",
            "data-mine": "{pass().verdict_of(\"Cody\").map_or(String::new(), |v| v.token().to_owned())}",
            "data-band": "{pass().consensus().map_or(String::new(), |v| v.token().to_owned())}",
        }
        session_ui::components::TakeReview {
            pass: pass(),
            performer: who(),
            performers: vec!["Cody".to_string(), "Joshua".to_string(), "Sarah".to_string()],
            on_performer: move |name: String| {
                let chosen = (!name.is_empty()).then_some(name);
                *performer().lock().expect("who") = chosen.clone();
                who.set(chosen);
            },
            on_mark: move |mark: Mark| {
                marked().lock().expect("marks").push(mark.clone());
                pass.with_mut(|pass| pass.mark(mark));
            },
        }
    }
}

async fn attribute(tester: &dioxus_test::DocumentTester, name: &str) -> String {
    tester
        .query(by_testid("debug"))
        .await
        .expect("debug element")
        .attribute(name)
        .unwrap_or_default()
}

/// A tablet nobody has claimed asks whose it is, and does not show a
/// take — rating without knowing who you are is a rating nobody can
/// use.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unclaimed_tablet_asks_who_is_holding_it() {
    *performer().lock().expect("who") = None;
    marked().lock().expect("marks").clear();

    let tester = render(Harness).build();
    // Awaited, which is what settles the first build; the absence check
    // after it is then a real absence rather than a race.
    tester
        .query(by_testid("who-are-you"))
        .await
        .expect("no picker on an unclaimed tablet");
    assert!(
        tester
            .query(by_testid("take-number"))
            .immediately()
            .is_err(),
        "it showed the take before anybody said who they were"
    );
    for name in ["Cody", "Joshua", "Sarah"] {
        tester
            .query(by_testid(format!("performer-{name}")))
            .await
            .unwrap_or_else(|_| panic!("{name} was not offered"));
    }
}

/// Once it is yours, the take is there with four ways to judge it, and
/// pressing one reports that verdict on the WHOLE take.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pressing_a_star_rates_the_whole_take() {
    *performer().lock().expect("who") = Some("Cody".to_string());
    marked().lock().expect("marks").clear();

    let tester = render(Harness).build();
    assert_eq!(
        tester
            .query(by_testid("take-number"))
            .await
            .expect("the take")
            .inner_html()
            .trim(),
        "Take 4"
    );
    // The band's existing verdict is visible before this player adds
    // theirs — you can see consensus forming.
    assert_eq!(attribute(&tester, "data-band").await, "2");
    assert_eq!(attribute(&tester, "data-mine").await, "");

    tester
        .query(by_testid("verdict-3"))
        .await
        .expect("a three-star button")
        .click();
    tester.pump().await.ok();

    let marks = marked().lock().expect("marks").clone();
    assert_eq!(marks.len(), 1, "{marks:?}");
    assert_eq!(marks[0].by, "Cody");
    assert_eq!(marks[0].verdict, Verdict::Amazing);
    assert!(
        marks[0].span.is_whole(LENGTH),
        "a rating with nothing selected was about a stretch: {:?}",
        marks[0].span
    );
    // And the band's verdict is still the worst of them.
    assert_eq!(attribute(&tester, "data-mine").await, "3");
    assert_eq!(attribute(&tester, "data-band").await, "2");
}

/// The X is a verdict like the others, and it outvotes them.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_x_is_a_verdict_and_it_wins() {
    *performer().lock().expect("who") = Some("Cody".to_string());
    marked().lock().expect("marks").clear();

    let tester = render(Harness).build();
    tester
        .query(by_testid("verdict-x"))
        .await
        .expect("the X")
        .click();
    tester.pump().await.ok();

    assert_eq!(attribute(&tester, "data-mine").await, "x");
    assert_eq!(
        attribute(&tester, "data-band").await,
        "x",
        "one mistake did not outvote the rave"
    );
}

/// Rating again corrects rather than piling up — you hit two and meant
/// three.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rating_twice_leaves_one_rating() {
    *performer().lock().expect("who") = Some("Cody".to_string());
    marked().lock().expect("marks").clear();

    let tester = render(Harness).build();
    for verdict in ["verdict-2", "verdict-3"] {
        tester
            .query(by_testid(verdict))
            .await
            .expect("a button")
            .click();
        tester.pump().await.ok();
    }
    // Joshua's plus Cody's one, not Cody's two.
    assert_eq!(attribute(&tester, "data-marks").await, "2");
    assert_eq!(attribute(&tester, "data-mine").await, "3");
}
