//! One peer's engine, kept in step with the shared doc.
//!
//! The bridge remembers the model the engine last showed. A local edit is
//! a difference between that and a fresh read of the engine, and goes into
//! the doc; a remote edit is a difference between that and the doc, and
//! goes into the engine. Either way the remembered model moves to the
//! engine's new state, so the engine never hears its own edit back and the
//! doc never records a remote edit twice.

use daw_control::Project;

use crate::diff::{Change, diff};
use crate::doc::{ORIGIN_LOCAL, SessionDoc};
use crate::engine::{self, Blobs, Direct, EngineIds, MediaRoot};
use crate::model::SessionModel;

pub struct Bridge {
    doc: SessionDoc,
    project: Project,
    media: MediaRoot,
    /// Where FX state is kept (the project's store).
    blobs: std::sync::Arc<dyn Blobs>,
    /// Which engine FX and item each doc FX and item is, here.
    ids: EngineIds,
    /// The engine's own services, when it runs in this process.
    direct: Option<std::sync::Arc<dyn Direct>>,
    /// What the engine showed after the last reconcile, chart included.
    shown: SessionModel,
}

impl Bridge {
    /// Start a session from this engine: the engine is the truth, the doc
    /// is written from it. `chart` is the song's chart text.
    ///
    /// # Errors
    /// When the engine cannot be read or the doc cannot be written.
    pub async fn host(
        project: Project,
        media: MediaRoot,
        doc: SessionDoc,
        chart: String,
        blobs: std::sync::Arc<dyn Blobs>,
        direct: Option<std::sync::Arc<dyn Direct>>,
    ) -> eyre::Result<Self> {
        let ids = EngineIds::default();
        let mut shown =
            engine::read(&project, &media, blobs.as_ref(), &ids, direct.as_deref()).await?;
        shown.chart = chart;
        doc.write(&shown, ORIGIN_LOCAL)?;
        Ok(Self {
            doc,
            project,
            media,
            blobs,
            ids,
            direct,
            shown,
        })
    }

    /// Join a session: `doc` already holds the host's state (imported
    /// before calling this), and the engine is made to match it —
    /// whatever it had open is replaced, never written into the doc.
    ///
    /// # Errors
    /// When the engine cannot be read or an engine call fails.
    pub async fn join(
        project: Project,
        media: MediaRoot,
        doc: SessionDoc,
        blobs: std::sync::Arc<dyn Blobs>,
        direct: Option<std::sync::Arc<dyn Direct>>,
    ) -> eyre::Result<Self> {
        let ids = EngineIds::default();
        let current =
            engine::read(&project, &media, blobs.as_ref(), &ids, direct.as_deref()).await?;
        let mut bridge = Self {
            doc,
            project,
            media,
            blobs,
            ids,
            direct,
            shown: current,
        };
        bridge.remote_changed().await?;
        Ok(bridge)
    }

    /// The shared doc — for the sync driver to import into and export from.
    #[must_use]
    pub const fn doc(&self) -> &SessionDoc {
        &self.doc
    }

    /// What the engine showed at the last reconcile.
    #[must_use]
    pub const fn shown(&self) -> &SessionModel {
        &self.shown
    }

    /// The engine may have changed locally: read it, and record any
    /// difference in the doc. Returns whether anything was recorded.
    ///
    /// # Errors
    /// When the engine cannot be read or the doc cannot be written.
    pub async fn local_changed(&mut self) -> eyre::Result<bool> {
        let mut now = engine::read(
            &self.project,
            &self.media,
            self.blobs.as_ref(),
            &self.ids,
            self.direct.as_deref(),
        )
        .await?;
        now.chart.clone_from(&self.shown.chart);
        if now == self.shown {
            return Ok(false);
        }
        self.doc.write(&now, ORIGIN_LOCAL)?;
        self.shown = now;
        Ok(true)
    }

    /// The engine finished an edit step named `label`: read it, and record
    /// what changed as one commit of that name — only what differs
    /// written. Returns whether anything was recorded.
    ///
    /// # Errors
    /// When the engine cannot be read or the doc cannot be written.
    pub async fn local_step(&mut self, label: &str) -> eyre::Result<bool> {
        let mut now = engine::read(
            &self.project,
            &self.media,
            self.blobs.as_ref(),
            &self.ids,
            self.direct.as_deref(),
        )
        .await?;
        now.chart.clone_from(&self.shown.chart);
        if now == self.shown {
            return Ok(false);
        }
        self.doc
            .write_step(&self.shown, &now, ORIGIN_LOCAL, label)?;
        self.shown = now;
        Ok(true)
    }

    /// The chart was edited locally.
    ///
    /// # Errors
    /// When the doc cannot be written.
    pub fn chart_changed(&mut self, chart: &str) -> eyre::Result<()> {
        if self.shown.chart == chart {
            return Ok(());
        }
        self.shown.chart = chart.to_string();
        self.doc.write(&self.shown, ORIGIN_LOCAL)?;
        Ok(())
    }

    /// The doc changed remotely: make the engine match it. Returns the
    /// steps taken — a caller redraws from them, and a `ChartChanged`
    /// among them is the chart editor's to show.
    ///
    /// # Errors
    /// When an engine call fails. The engine is re-read either way, so a
    /// half-applied change is reconciled on the next call.
    pub async fn remote_changed(&mut self) -> eyre::Result<Vec<Change>> {
        let target = self.doc.read();
        let changes = diff(&self.shown, &target);
        if changes.is_empty() {
            return Ok(changes);
        }
        let applied = engine::apply(
            &self.project,
            &self.media,
            &changes,
            &target,
            self.blobs.as_ref(),
            &mut self.ids,
            self.direct.as_deref(),
        )
        .await;
        let mut now = engine::read(
            &self.project,
            &self.media,
            self.blobs.as_ref(),
            &self.ids,
            self.direct.as_deref(),
        )
        .await?;
        now.chart = target.chart;
        self.shown = now;
        applied?;
        Ok(changes)
    }
}
