//! Live in-process implementations of the Keyflow proto contracts.
//!
//! `keyflow-proto` defines the domain model and RPC traits. This crate is the
//! concrete local implementation used by apps, CLIs, and tests that do not need
//! to cross a process boundary.

#![deny(unsafe_code)]

use std::collections::HashMap;

use keyflow_proto::{
    ChartParsers, Charts, GuideRequest, Guides, ParseError, ParseRequest, ParseResponse, Parsers,
};

/// In-memory chart implementation.
#[derive(Default, Clone)]
pub struct Chart {
    charts: HashMap<String, keyflow_proto::Chart>,
}

impl Chart {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_charts(charts: HashMap<String, keyflow_proto::Chart>) -> Self {
        Self { charts }
    }

    pub fn insert(&mut self, chart_id: impl Into<String>, chart: keyflow_proto::Chart) {
        self.charts.insert(chart_id.into(), chart);
    }
}

impl Charts for Chart {
    fn get_chart(&self, chart_id: String) -> Option<keyflow_proto::Chart> {
        self.charts.get(&chart_id).cloned()
    }

    fn parse(&self, text: String) -> Result<keyflow_proto::Chart, ParseError> {
        parse_text_chart(&text)
    }
}

/// Text and MIDI chart parser implementation.
#[derive(Debug, Default, Clone, Copy)]
pub struct ChartParser;

impl ChartParser {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl ChartParsers for ChartParser {
    fn parse_text(&self, text: String) -> Result<keyflow_proto::Chart, String> {
        keyflow_text::chart::parse_chart(&text)
    }

    fn parse_midi(&self, bytes: Vec<u8>) -> Result<keyflow_proto::Chart, String> {
        keyflow_midi::parse_midi_bytes(&bytes)
    }
}

/// Parse request/response implementation.
#[derive(Debug, Default, Clone, Copy)]
pub struct Parser;

impl Parser {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Parsers for Parser {
    fn parse_chart(&self, request: ParseRequest) -> Result<ParseResponse, ParseError> {
        Ok(ParseResponse {
            chart: parse_text_chart(&request.text)?,
        })
    }
}

/// Guide event generator implementation.
#[derive(Debug, Default, Clone, Copy)]
pub struct Guide;

impl Guide {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Guides for Guide {
    fn generate_count_in_events(&self, request: GuideRequest) -> Vec<keyflow_proto::GuideEvent> {
        keyflow_midi::guide::GuideGenerator::generate(
            request.section_start_quarters,
            request.count_in_start_quarters,
            &request.time_signature,
            request.tempo,
            &request.section_type,
            request.section_number,
            &request.click_config,
            &request.count_in_config,
            &request.guide_config,
        )
    }
}

fn parse_text_chart(text: &str) -> Result<keyflow_proto::Chart, ParseError> {
    keyflow_text::chart::parse_chart(text).map_err(ParseError::InvalidSyntax)
}
