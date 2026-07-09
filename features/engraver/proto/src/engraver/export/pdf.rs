//! PDF export for the scene graph.
//!
//! Converts SceneNode trees into PDF documents using printpdf.
//! Produces high-quality vector PDFs similar to LilyPond output.
//!
//! This module is WASM compatible thanks to printpdf's pure Rust implementation.
//!
//! ## Font Embedding
//!
//! The PDF exporter supports embedding custom fonts (TTF/OTF) for text and music symbols.
//! When fonts are provided via `PdfExportConfig`, they are parsed and embedded in the PDF,
//! ensuring the output looks identical regardless of system fonts.
//!
//! ## SVG-to-PDF Export
//!
//! For highest fidelity output, the `serialize_from_svg` method converts SVG strings
//! directly to PDF using the svg2pdf crate. This preserves all vector graphics and
//! text exactly as rendered, with fonts embedded from provided font data.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use kurbo::{Affine, Shape};
use peniko::Color;
use printpdf::{
    BuiltinFont, Color as PdfColor, FontId, LinePoint, Mm, Op, ParsedFont, PdfDocument, PdfPage,
    PdfSaveOptions, PdfWarnMsg, Point as PdfPoint, PolygonRing, Pt, TextItem,
};
use thiserror::Error;

use crate::engraver::scene::node::SceneNode;
use crate::engraver::scene::paint::{FontStyle, FontWeight, LineCap, PaintCommand, TextAnchor};

/// Errors that can occur during PDF export.
#[derive(Debug, Error)]
pub enum PdfExportError {
    /// Failed to save PDF file
    #[error("Failed to save PDF: {0}")]
    SaveError(String),

    /// Failed to add custom font
    #[error("Failed to add font: {0}")]
    FontError(String),

    /// IO error
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    /// SVG parsing error
    #[error("Failed to parse SVG: {0}")]
    SvgParseError(String),
}

/// Configuration for PDF export.
#[derive(Debug, Clone)]
pub struct PdfExportConfig {
    /// Page width in points (1 point = 1/72 inch)
    pub width: f64,
    /// Page height in points
    pub height: f64,
    /// Document title
    pub title: String,
    /// Document author
    pub author: Option<String>,
    /// Background color (None for white)
    pub background: Option<Color>,
    /// Default stroke width
    pub default_stroke_width: f64,
    /// Font data for text rendering (optional)
    pub font_data: Option<Arc<Vec<u8>>>,
    /// SMuFL font data for music symbols (optional)
    pub smufl_font_data: Option<Arc<Vec<u8>>>,
}

impl Default for PdfExportConfig {
    fn default() -> Self {
        Self {
            width: 612.0,  // US Letter width in points (8.5" × 72)
            height: 792.0, // US Letter height in points (11" × 72)
            title: "Untitled".to_string(),
            author: None,
            background: None,
            default_stroke_width: 0.5,
            font_data: None,
            smufl_font_data: None,
        }
    }
}

impl PdfExportConfig {
    /// Create a config for A4 paper size.
    #[must_use]
    pub fn a4() -> Self {
        Self {
            width: 595.0,  // A4 width in points (210mm)
            height: 842.0, // A4 height in points (297mm)
            ..Default::default()
        }
    }

    /// Create a config for US Letter paper size.
    #[must_use]
    pub fn letter() -> Self {
        Self::default()
    }

    /// Set the document title.
    #[must_use]
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    /// Set the document author.
    #[must_use]
    pub fn with_author(mut self, author: impl Into<String>) -> Self {
        self.author = Some(author.into());
        self
    }

    /// Set the background color.
    #[must_use]
    pub fn with_background(mut self, color: Color) -> Self {
        self.background = Some(color);
        self
    }

    /// Set the text font data.
    #[must_use]
    pub fn with_font(mut self, font_data: Arc<Vec<u8>>) -> Self {
        self.font_data = Some(font_data);
        self
    }

    /// Set the SMuFL font data for music symbols.
    #[must_use]
    pub fn with_smufl_font(mut self, font_data: Arc<Vec<u8>>) -> Self {
        self.smufl_font_data = Some(font_data);
        self
    }
}

/// PDF serializer that converts scene graphs to PDF documents.
pub struct PdfSerializer {
    config: PdfExportConfig,
    /// Parsed text font (if provided)
    text_font: Option<ParsedFont>,
    /// FontId for text font
    text_font_id: Option<FontId>,
    /// Parsed SMuFL font (if provided)
    smufl_font: Option<ParsedFont>,
    /// FontId for SMuFL font
    smufl_font_id: Option<FontId>,
    /// Cache of named fonts (font_family -> (ParsedFont, FontId))
    named_fonts: HashMap<String, (ParsedFont, FontId)>,
}

impl PdfSerializer {
    /// Create a new PDF serializer with the given configuration.
    #[must_use]
    pub fn new(config: PdfExportConfig) -> Self {
        let mut warnings = Vec::new();

        // Parse text font if provided
        let (text_font, text_font_id) = if let Some(ref font_data) = config.font_data {
            if let Some(font) = ParsedFont::from_bytes(font_data, 0, &mut warnings) {
                let font_id = FontId::new();
                (Some(font), Some(font_id))
            } else {
                (None, None)
            }
        } else {
            (None, None)
        };

        // Parse SMuFL font if provided
        let (smufl_font, smufl_font_id) = if let Some(ref font_data) = config.smufl_font_data {
            if let Some(font) = ParsedFont::from_bytes(font_data, 0, &mut warnings) {
                let font_id = FontId::new();
                (Some(font), Some(font_id))
            } else {
                (None, None)
            }
        } else {
            (None, None)
        };

        Self {
            config,
            text_font,
            text_font_id,
            smufl_font,
            smufl_font_id,
            named_fonts: HashMap::new(),
        }
    }

    /// Add a named font for use with specific font families.
    pub fn add_named_font(&mut self, name: &str, font_data: &[u8]) -> Result<(), PdfExportError> {
        let mut warnings = Vec::new();
        let font = ParsedFont::from_bytes(font_data, 0, &mut warnings)
            .ok_or_else(|| PdfExportError::FontError("Failed to parse font".to_string()))?;
        let font_id = FontId::new();
        self.named_fonts.insert(name.to_string(), (font, font_id));
        Ok(())
    }

    /// Serialize a scene graph to PDF bytes.
    pub fn serialize(&mut self, scene: &SceneNode) -> Result<Vec<u8>, PdfExportError> {
        let mut doc = PdfDocument::new(&self.config.title);

        // Register fonts with the document
        if let (Some(font), Some(font_id)) = (&self.text_font, &self.text_font_id) {
            doc.resources
                .fonts
                .map
                .insert(font_id.clone(), font.clone());
        }
        if let (Some(font), Some(font_id)) = (&self.smufl_font, &self.smufl_font_id) {
            doc.resources
                .fonts
                .map
                .insert(font_id.clone(), font.clone());
        }
        for (font, font_id) in self.named_fonts.values() {
            doc.resources
                .fonts
                .map
                .insert(font_id.clone(), font.clone());
        }

        // Collect all operations for the page
        let mut ops = Vec::new();

        // Draw background if set
        if let Some(bg) = self.config.background {
            self.add_background(&mut ops, bg);
        }

        // Render scene content
        self.render_node(&mut ops, scene, Affine::IDENTITY);

        // Create page with collected operations
        // Convert points to mm for printpdf (1 point = 0.352778 mm)
        let width_mm = Mm((self.config.width * 0.352778) as f32);
        let height_mm = Mm((self.config.height * 0.352778) as f32);
        let page = PdfPage::new(width_mm, height_mm, ops);

        // Save to bytes
        let save_options = PdfSaveOptions::default();
        let mut warnings = Vec::<PdfWarnMsg>::new();
        let bytes = doc
            .with_pages(vec![page])
            .save(&save_options, &mut warnings);

        Ok(bytes)
    }

    /// Create a PDF from a pre-rendered PNG image.
    ///
    /// This method creates a PDF by embedding a rasterized image rather than
    /// converting the scene graph to vector operations. This guarantees identical
    /// output to the Vello render but produces a larger file size.
    ///
    /// # Arguments
    /// * `png_bytes` - PNG-encoded image data
    /// * `dpi` - The DPI at which the image was rendered (for proper scaling)
    ///
    /// # Returns
    /// PDF bytes ready to be saved or transmitted.
    pub fn serialize_from_png(
        &self,
        png_bytes: &[u8],
        dpi: f64,
    ) -> Result<Vec<u8>, PdfExportError> {
        use printpdf::{RawImage, XObjectTransform};

        let mut doc = PdfDocument::new(&self.config.title);

        // Decode PNG image
        let mut warnings = Vec::new();
        let image = RawImage::decode_from_bytes(png_bytes, &mut warnings)
            .map_err(|e| PdfExportError::SaveError(format!("Failed to decode PNG: {e}")))?;

        // Get image dimensions in pixels
        let image_width_px = image.width as f64;
        let image_height_px = image.height as f64;

        // Add image to document resources
        let image_id = doc.add_image(&image);

        // Scale image to fill the page exactly
        // In PDF, images are placed in user space units (points)
        // scale_x/scale_y convert pixels to points
        let scale_x = self.config.width / image_width_px;
        let scale_y = self.config.height / image_height_px;

        // Position image at bottom-left (0,0) - it will extend up and right
        // With proper scaling, top of image will reach top of page
        let transform = XObjectTransform {
            translate_x: Some(Pt(0.0)),
            translate_y: Some(Pt(0.0)),
            scale_x: Some(scale_x as f32),
            scale_y: Some(scale_y as f32),
            ..Default::default()
        };

        // Ignore DPI parameter - we scale to fit page exactly
        let _ = dpi;

        // Create page with the image
        let width_mm = Mm((self.config.width * 0.352778) as f32);
        let height_mm = Mm((self.config.height * 0.352778) as f32);

        let ops = vec![printpdf::Op::UseXobject {
            id: image_id,
            transform,
        }];

        let page = PdfPage::new(width_mm, height_mm, ops);

        // Save to bytes
        let save_options = PdfSaveOptions::default();
        let mut save_warnings = Vec::<PdfWarnMsg>::new();
        let bytes = doc
            .with_pages(vec![page])
            .save(&save_options, &mut save_warnings);

        Ok(bytes)
    }

    /// Create a multi-page PDF from pre-rendered PNG images.
    ///
    /// Each page can have different dimensions. Images are scaled to fill their
    /// respective pages exactly.
    ///
    /// # Arguments
    /// * `pages` - Vector of (png_bytes, page_width_pts, page_height_pts) for each page
    ///
    /// # Returns
    /// PDF bytes ready to be saved or transmitted.
    pub fn serialize_multi_page_from_png(
        pages: &[(Vec<u8>, f64, f64)],
    ) -> Result<Vec<u8>, PdfExportError> {
        use printpdf::{RawImage, XObjectTransform};

        if pages.is_empty() {
            return Err(PdfExportError::SaveError("No pages to export".to_string()));
        }

        let mut doc = PdfDocument::new("Chart Export");
        let mut pdf_pages = Vec::new();

        for (png_bytes, page_width, page_height) in pages {
            // Decode PNG image
            let mut warnings = Vec::new();
            let image = RawImage::decode_from_bytes(png_bytes, &mut warnings)
                .map_err(|e| PdfExportError::SaveError(format!("Failed to decode PNG: {e}")))?;

            // Get image dimensions in pixels
            let image_width_px = image.width as f64;
            let image_height_px = image.height as f64;

            // Add image to document resources
            let image_id = doc.add_image(&image);

            // Scale image to fill the page exactly
            let scale_x = page_width / image_width_px;
            let scale_y = page_height / image_height_px;

            let transform = XObjectTransform {
                translate_x: Some(Pt(0.0)),
                translate_y: Some(Pt(0.0)),
                scale_x: Some(scale_x as f32),
                scale_y: Some(scale_y as f32),
                ..Default::default()
            };

            // Create page with the image
            let width_mm = Mm((*page_width * 0.352778) as f32);
            let height_mm = Mm((*page_height * 0.352778) as f32);

            let ops = vec![printpdf::Op::UseXobject {
                id: image_id,
                transform,
            }];

            pdf_pages.push(PdfPage::new(width_mm, height_mm, ops));
        }

        // Save to bytes
        let save_options = PdfSaveOptions::default();
        let mut save_warnings = Vec::<PdfWarnMsg>::new();
        let bytes = doc
            .with_pages(pdf_pages)
            .save(&save_options, &mut save_warnings);

        Ok(bytes)
    }

    /// Create a multi-page PDF from SVG strings using svg2pdf.
    ///
    /// This method provides the highest fidelity output by converting SVG documents
    /// directly to PDF vector graphics. All text remains as vector paths with
    /// embedded fonts, preserving exact rendering quality.
    ///
    /// # Arguments
    /// * `svg_pages` - Vector of SVG strings, one per page
    /// * `fonts` - Font data to embed (name -> bytes). Should include:
    ///   - "Bravura" or similar for SMuFL music symbols
    ///   - "MuseJazzText" for chord symbols
    ///   - "FreeSans" or similar for general text
    ///
    /// # Returns
    /// PDF bytes ready to be saved or transmitted.
    ///
    /// # Example
    /// ```ignore
    /// let fonts = vec![
    ///     ("Bravura", bravura_bytes.as_slice()),
    ///     ("MuseJazzText", musejazz_bytes.as_slice()),
    ///     ("FreeSans", freesans_bytes.as_slice()),
    /// ];
    /// let pdf = PdfSerializer::serialize_from_svg(&svg_pages, &fonts)?;
    /// ```
    pub fn serialize_from_svg(
        svg_pages: &[String],
        fonts: &[(&str, &[u8])],
    ) -> Result<Vec<u8>, PdfExportError> {
        use svg2pdf::{ConversionOptions, PageOptions};

        if svg_pages.is_empty() {
            return Err(PdfExportError::SaveError("No pages to export".to_string()));
        }

        // Build font database with provided fonts
        // Use usvg's re-exported fontdb to ensure version compatibility
        let mut fontdb = usvg::fontdb::Database::new();
        for (name, data) in fonts {
            // Load font data - fontdb will parse and register all faces
            fontdb.load_font_data(data.to_vec());
            tracing::debug!("Loaded font '{}' ({} bytes)", name, data.len());
        }

        // Log available font families for debugging
        for face in fontdb.faces() {
            tracing::debug!(
                "Font face available: {} ({})",
                face.families
                    .first()
                    .map(|(name, _)| name.as_str())
                    .unwrap_or("unknown"),
                face.post_script_name
            );
        }

        // Convert each SVG page to PDF bytes
        let mut pdf_chunks: Vec<Vec<u8>> = Vec::new();

        for (i, svg_str) in svg_pages.iter().enumerate() {
            // Parse SVG with usvg
            let mut options = usvg::Options::default();
            // Use our custom fontdb instead of system fonts
            *options.fontdb_mut() = fontdb.clone();

            let tree = usvg::Tree::from_str(svg_str, &options)
                .map_err(|e| PdfExportError::SvgParseError(format!("Page {}: {}", i + 1, e)))?;

            // Convert to PDF
            let pdf = svg2pdf::to_pdf(&tree, ConversionOptions::default(), PageOptions::default())
                .map_err(|e| {
                    PdfExportError::SaveError(format!("SVG to PDF conversion failed: {e}"))
                })?;

            pdf_chunks.push(pdf);
        }

        // If single page, return directly
        if pdf_chunks.len() == 1 {
            return Ok(pdf_chunks.into_iter().next().unwrap());
        }

        // For multiple pages, we need to combine them
        // svg2pdf creates standalone PDFs, so for multi-page we use a different approach:
        // Convert each SVG to a PDF XObject and place on pages
        Self::combine_svg_pdfs_to_multipage(svg_pages, fonts)
    }

    /// Combine multiple SVG documents into a single multi-page PDF.
    ///
    /// Uses svg2pdf's to_pdf to create complete PDFs for each page,
    /// then merges them using lopdf.
    fn combine_svg_pdfs_to_multipage(
        svg_pages: &[String],
        fonts: &[(&str, &[u8])],
    ) -> Result<Vec<u8>, PdfExportError> {
        use lopdf::{Document, Object, ObjectId, dictionary};
        use std::collections::BTreeMap;
        use svg2pdf::{ConversionOptions, PageOptions};

        tracing::debug!(
            "combine_svg_pdfs_to_multipage: starting with {} pages",
            svg_pages.len()
        );

        // Build font database using usvg's re-exported fontdb
        let mut fontdb = usvg::fontdb::Database::new();
        for (name, data) in fonts {
            fontdb.load_font_data(data.to_vec());
            tracing::debug!("Loaded font '{}' ({} bytes)", name, data.len());
        }

        // Log all font families in the database
        let mut families: Vec<String> = Vec::new();
        for face in fontdb.faces() {
            for family in face.families.iter() {
                if !families.contains(&family.0) {
                    families.push(family.0.clone());
                }
            }
        }
        tracing::debug!("Font database contains families: {:?}", families);

        // Configure generic font family fallbacks
        // Map generic family names to actual loaded fonts
        let text_font = if families.contains(&"FreeSans".to_string()) {
            "FreeSans"
        } else if families.contains(&"Free Sans".to_string()) {
            "Free Sans"
        } else {
            families.first().map(|s| s.as_str()).unwrap_or("FreeSans")
        };

        fontdb.set_sans_serif_family(text_font);
        fontdb.set_serif_family(text_font); // Use same font since we don't have a serif
        tracing::debug!("Set sans-serif and serif fallback to '{}'", text_font);

        // Step 1: Generate a complete PDF for each SVG page using to_pdf
        let mut pdf_documents: Vec<Document> = Vec::new();

        let font_family_re = regex::Regex::new(r#"font-family="([^"]+)""#).unwrap();

        for (page_idx, svg_str) in svg_pages.iter().enumerate() {
            tracing::debug!("Processing SVG page {}", page_idx + 1);

            if svg_str.is_empty() {
                return Err(PdfExportError::SvgParseError(format!(
                    "Page {} has empty SVG content",
                    page_idx + 1
                )));
            }

            // Log font-family references in the SVG (first page only for brevity)
            if page_idx == 0 {
                let mut svg_fonts: Vec<String> = Vec::new();
                for cap in font_family_re.captures_iter(svg_str) {
                    let font = cap.get(1).unwrap().as_str().to_string();
                    if !svg_fonts.contains(&font) {
                        svg_fonts.push(font);
                    }
                }
                tracing::debug!("SVG references font families: {:?}", svg_fonts);
            }

            let mut options = usvg::Options::default();
            *options.fontdb_mut() = fontdb.clone();

            tracing::debug!("Parsing SVG with usvg...");
            let tree = usvg::Tree::from_str(svg_str, &options).map_err(|e| {
                PdfExportError::SvgParseError(format!("Page {}: {}", page_idx + 1, e))
            })?;

            let size = tree.size();
            tracing::debug!(
                "PDF export page {}: usvg tree size {}x{}",
                page_idx + 1,
                size.width(),
                size.height()
            );

            // Convert to complete PDF using to_pdf
            tracing::debug!("Converting to PDF...");
            let pdf_bytes =
                svg2pdf::to_pdf(&tree, ConversionOptions::default(), PageOptions::default())
                    .map_err(|e| PdfExportError::SaveError(format!("SVG to PDF failed: {e}")))?;

            // Parse the PDF with lopdf
            let doc = Document::load_mem(&pdf_bytes)
                .map_err(|e| PdfExportError::SaveError(format!("Failed to parse PDF: {e}")))?;

            pdf_documents.push(doc);
            tracing::debug!(
                "Page {} PDF created ({} bytes)",
                page_idx + 1,
                pdf_bytes.len()
            );
        }

        // Step 2: Merge all PDFs using lopdf pattern
        tracing::debug!("Merging {} PDFs...", pdf_documents.len());

        if pdf_documents.is_empty() {
            return Err(PdfExportError::SaveError("No pages to merge".to_string()));
        }

        // Renumber objects in each document to avoid ID conflicts
        let mut max_id = 1u32;
        let mut all_pages: Vec<ObjectId> = Vec::new();
        let mut all_objects: BTreeMap<ObjectId, Object> = BTreeMap::new();

        for mut doc in pdf_documents {
            // Renumber all objects in this document
            doc.renumber_objects_with(max_id);
            max_id = doc.max_id + 1;

            // Extract pages from this document
            let pages = doc.get_pages();
            for (_, page_id) in pages {
                all_pages.push(page_id);
            }

            // Collect all objects (excluding Catalog, Pages, and Outlines)
            for (id, object) in doc.objects {
                if let Ok(type_name) = object.type_name()
                    && (type_name == b"Catalog" || type_name == b"Outlines")
                {
                    continue; // Skip catalog and outlines, we'll create our own
                }
                all_objects.insert(id, object);
            }
        }

        tracing::debug!(
            "Collected {} pages and {} objects",
            all_pages.len(),
            all_objects.len()
        );

        // Build merged document
        let mut merged_doc = Document::with_version("1.5");

        // Add all collected objects
        for (id, object) in all_objects {
            merged_doc.objects.insert(id, object);
        }

        // Create Pages object
        let pages_id = (max_id, 0);
        max_id += 1;

        // Update each page's Parent to point to our new Pages object
        for page_id in &all_pages {
            if let Ok(page_obj) = merged_doc.get_object_mut(*page_id)
                && let Object::Dictionary(dict) = page_obj
            {
                dict.set("Parent", Object::Reference(pages_id));
            }
        }

        // Create Pages dictionary
        let kids: Vec<Object> = all_pages.iter().map(|id| Object::Reference(*id)).collect();
        let pages_dict = dictionary! {
            "Type" => "Pages",
            "Count" => all_pages.len() as i64,
            "Kids" => kids,
        };
        merged_doc
            .objects
            .insert(pages_id, Object::Dictionary(pages_dict));

        // Create Catalog
        let catalog_id = (max_id, 0);
        let catalog_dict = dictionary! {
            "Type" => "Catalog",
            "Pages" => Object::Reference(pages_id),
        };
        merged_doc
            .objects
            .insert(catalog_id, Object::Dictionary(catalog_dict));

        // Set trailer
        merged_doc
            .trailer
            .set("Root", Object::Reference(catalog_id));
        merged_doc.max_id = max_id;

        // Save merged document to bytes
        tracing::debug!("Saving merged PDF...");
        let mut output = Vec::new();
        merged_doc
            .save_to(&mut output)
            .map_err(|e| PdfExportError::SaveError(format!("Failed to save PDF: {e}")))?;

        tracing::debug!("Merged PDF complete: {} bytes", output.len());
        Ok(output)
    }

    /// Serialize a scene graph and save directly to a file.
    pub fn save_to_file(
        &mut self,
        scene: &SceneNode,
        path: impl AsRef<Path>,
    ) -> Result<(), PdfExportError> {
        let bytes = self.serialize(scene)?;
        std::fs::write(path, bytes)?;
        Ok(())
    }

    /// Add background rectangle operations.
    fn add_background(&self, ops: &mut Vec<Op>, color: Color) {
        let pdf_color = vello_to_pdf_color(color);
        ops.push(Op::SaveGraphicsState);
        ops.push(Op::SetFillColor { col: pdf_color });

        // Draw rectangle covering entire page
        let points = vec![
            LinePoint {
                p: PdfPoint {
                    x: Pt(0.0),
                    y: Pt(0.0),
                },
                bezier: false,
            },
            LinePoint {
                p: PdfPoint {
                    x: Pt(self.config.width as f32),
                    y: Pt(0.0),
                },
                bezier: false,
            },
            LinePoint {
                p: PdfPoint {
                    x: Pt(self.config.width as f32),
                    y: Pt(self.config.height as f32),
                },
                bezier: false,
            },
            LinePoint {
                p: PdfPoint {
                    x: Pt(0.0),
                    y: Pt(self.config.height as f32),
                },
                bezier: false,
            },
        ];

        ops.push(Op::DrawPolygon {
            polygon: printpdf::Polygon {
                rings: vec![PolygonRing { points }],
                mode: printpdf::PaintMode::Fill,
                winding_order: printpdf::WindingOrder::NonZero,
            },
        });
        ops.push(Op::RestoreGraphicsState);
    }

    /// Render a scene node and its children.
    fn render_node(&self, ops: &mut Vec<Op>, node: &SceneNode, parent_transform: Affine) {
        if !node.visible {
            return;
        }

        let combined_transform = parent_transform * node.transform;

        // Render paint commands
        for cmd in &node.commands {
            self.render_paint_command(ops, cmd, &combined_transform);
        }

        // Render children
        for child in &node.children {
            self.render_node(ops, child, combined_transform);
        }
    }

    /// Render a paint command to PDF operations.
    fn render_paint_command(&self, ops: &mut Vec<Op>, cmd: &PaintCommand, transform: &Affine) {
        match cmd {
            PaintCommand::Fill { path, color, .. } => {
                self.render_filled_path(ops, path, *color, transform);
            }

            PaintCommand::Stroke {
                path, color, width, ..
            } => {
                self.render_stroked_path(ops, path, *color, *width, transform);
            }

            PaintCommand::Glyph {
                codepoint,
                position,
                size,
                color,
            } => {
                let transformed = transform_point(transform, *position);
                self.render_glyph(ops, *codepoint, transformed, *size, *color);
            }

            PaintCommand::Text {
                text,
                font_family,
                font_size,
                position,
                color,
                anchor,
                weight,
                style,
            } => {
                let transformed = transform_point(transform, *position);
                self.render_text(
                    ops,
                    text,
                    font_family,
                    *font_size,
                    transformed,
                    *color,
                    *anchor,
                    *weight,
                    *style,
                );
            }

            PaintCommand::Line {
                start,
                end,
                width,
                color,
                line_cap,
            } => {
                let start_t = transform_point(transform, *start);
                let end_t = transform_point(transform, *end);
                self.render_line(ops, start_t, end_t, *width, *color, *line_cap);
            }

            PaintCommand::Rect {
                rect,
                fill,
                stroke,
                stroke_width,
                corner_radius,
            } => {
                self.render_rect(
                    ops,
                    rect,
                    transform,
                    *fill,
                    *stroke,
                    *stroke_width,
                    corner_radius.unwrap_or(0.0),
                );
            }

            PaintCommand::Circle {
                center,
                radius,
                fill,
                stroke,
                stroke_width,
            } => {
                let center_t = transform_point(transform, *center);
                self.render_circle(ops, center_t, *radius, *fill, *stroke, *stroke_width);
            }

            PaintCommand::Ellipse {
                center,
                radius_x,
                radius_y,
                fill,
                stroke,
                stroke_width,
            } => {
                let center_t = transform_point(transform, *center);
                self.render_ellipse(
                    ops,
                    center_t,
                    *radius_x,
                    *radius_y,
                    *fill,
                    *stroke,
                    *stroke_width,
                );
            }
        }
    }

    /// Render a filled path.
    fn render_filled_path(
        &self,
        ops: &mut Vec<Op>,
        path: &kurbo::BezPath,
        color: Color,
        transform: &Affine,
    ) {
        let pdf_color = vello_to_pdf_color(color);
        ops.push(Op::SaveGraphicsState);
        ops.push(Op::SetFillColor { col: pdf_color });

        // Convert path to polygon points
        let points = bezpath_to_line_points(path, transform, self.config.height);

        if !points.is_empty() {
            ops.push(Op::DrawPolygon {
                polygon: printpdf::Polygon {
                    rings: vec![PolygonRing { points }],
                    mode: printpdf::PaintMode::Fill,
                    winding_order: printpdf::WindingOrder::NonZero,
                },
            });
        }

        ops.push(Op::RestoreGraphicsState);
    }

    /// Render a stroked path.
    fn render_stroked_path(
        &self,
        ops: &mut Vec<Op>,
        path: &kurbo::BezPath,
        color: Color,
        width: f64,
        transform: &Affine,
    ) {
        let pdf_color = vello_to_pdf_color(color);
        ops.push(Op::SaveGraphicsState);
        ops.push(Op::SetOutlineColor { col: pdf_color });
        ops.push(Op::SetOutlineThickness {
            pt: Pt(width as f32),
        });

        // Convert path to line points
        let points = bezpath_to_line_points(path, transform, self.config.height);

        if points.len() >= 2 {
            ops.push(Op::DrawLine {
                line: printpdf::Line {
                    points,
                    is_closed: false,
                },
            });
        }

        ops.push(Op::RestoreGraphicsState);
    }

    /// Render a music glyph (SMuFL).
    ///
    /// Note: SMuFL fonts are designed so that 1 em = 4 staff spaces (spatium).
    /// The layout system passes `spatium` as the glyph size, so we multiply by 4
    /// to get the correct font size for rendering.
    fn render_glyph(
        &self,
        ops: &mut Vec<Op>,
        codepoint: char,
        position: kurbo::Point,
        size: f64,
        color: Color,
    ) {
        let pdf_color = vello_to_pdf_color(color);

        // Flip Y coordinate for PDF (origin at bottom-left)
        let y = self.config.height - position.y;

        // SMuFL: 1 em = 4 staff spaces, so font_size = spatium * 4
        let font_size = size * 4.0;

        ops.push(Op::SaveGraphicsState);
        ops.push(Op::SetFillColor { col: pdf_color });
        ops.push(Op::StartTextSection);
        ops.push(Op::SetTextCursor {
            pos: PdfPoint {
                x: Pt(position.x as f32),
                y: Pt(y as f32),
            },
        });

        // Use embedded SMuFL font if available, otherwise fall back to builtin Symbol
        if let Some(font_id) = &self.smufl_font_id {
            ops.push(Op::SetFontSize {
                size: Pt(font_size as f32),
                font: font_id.clone(),
            });

            // Use WriteText to let printpdf handle character-to-glyph mapping via cmap
            // SMuFL fonts have proper cmap entries for Unicode PUA codepoints
            ops.push(Op::WriteText {
                items: vec![TextItem::Text(codepoint.to_string())],
                font: font_id.clone(),
            });
        } else {
            // Fallback to builtin Symbol font (limited support)
            ops.push(Op::SetFontSizeBuiltinFont {
                size: Pt(font_size as f32),
                font: BuiltinFont::Symbol,
            });
            ops.push(Op::WriteTextBuiltinFont {
                items: vec![TextItem::Text(codepoint.to_string())],
                font: BuiltinFont::Symbol,
            });
        }

        ops.push(Op::EndTextSection);
        ops.push(Op::RestoreGraphicsState);
    }

    /// Render text.
    #[allow(clippy::too_many_arguments)]
    fn render_text(
        &self,
        ops: &mut Vec<Op>,
        text: &str,
        font_family: &str,
        font_size: f64,
        position: kurbo::Point,
        color: Color,
        anchor: TextAnchor,
        weight: FontWeight,
        _style: FontStyle,
    ) {
        let pdf_color = vello_to_pdf_color(color);

        // Flip Y coordinate for PDF
        let y = self.config.height - position.y;

        // Adjust x position based on anchor (approximate)
        let x = match anchor {
            TextAnchor::Start => position.x,
            TextAnchor::Middle => position.x - (text.len() as f64 * font_size * 0.3),
            TextAnchor::End => position.x - (text.len() as f64 * font_size * 0.6),
        };

        ops.push(Op::SaveGraphicsState);
        ops.push(Op::SetFillColor { col: pdf_color });
        ops.push(Op::StartTextSection);
        ops.push(Op::SetTextCursor {
            pos: PdfPoint {
                x: Pt(x as f32),
                y: Pt(y as f32),
            },
        });

        // Try to find a named font matching the font_family, or use the default text font
        let font_id_to_use = self
            .named_fonts
            .get(font_family)
            .map(|(_, id)| id.clone())
            .or_else(|| self.text_font_id.clone());

        if let Some(font_id) = font_id_to_use {
            // Use embedded font with WriteText (handles character mapping automatically)
            ops.push(Op::SetFontSize {
                size: Pt(font_size as f32),
                font: font_id.clone(),
            });

            // WriteText handles character-to-glyph mapping internally
            ops.push(Op::WriteText {
                items: vec![TextItem::Text(text.to_string())],
                font: font_id,
            });
        } else {
            // Fallback to builtin font
            let font = match weight {
                FontWeight::Bold => BuiltinFont::HelveticaBold,
                _ => BuiltinFont::Helvetica,
            };

            ops.push(Op::SetFontSizeBuiltinFont {
                size: Pt(font_size as f32),
                font,
            });
            ops.push(Op::WriteTextBuiltinFont {
                items: vec![TextItem::Text(text.to_string())],
                font,
            });
        }

        ops.push(Op::EndTextSection);
        ops.push(Op::RestoreGraphicsState);
    }

    /// Render a line.
    fn render_line(
        &self,
        ops: &mut Vec<Op>,
        start: kurbo::Point,
        end: kurbo::Point,
        width: f64,
        color: Color,
        _line_cap: LineCap,
    ) {
        let pdf_color = vello_to_pdf_color(color);

        // Flip Y coordinates for PDF
        let start_y = self.config.height - start.y;
        let end_y = self.config.height - end.y;

        ops.push(Op::SaveGraphicsState);
        ops.push(Op::SetOutlineColor { col: pdf_color });
        ops.push(Op::SetOutlineThickness {
            pt: Pt(width as f32),
        });
        ops.push(Op::DrawLine {
            line: printpdf::Line {
                points: vec![
                    LinePoint {
                        p: PdfPoint {
                            x: Pt(start.x as f32),
                            y: Pt(start_y as f32),
                        },
                        bezier: false,
                    },
                    LinePoint {
                        p: PdfPoint {
                            x: Pt(end.x as f32),
                            y: Pt(end_y as f32),
                        },
                        bezier: false,
                    },
                ],
                is_closed: false,
            },
        });
        ops.push(Op::RestoreGraphicsState);
    }

    /// Render a rectangle, optionally with rounded corners.
    #[allow(clippy::too_many_arguments)]
    fn render_rect(
        &self,
        ops: &mut Vec<Op>,
        rect: &kurbo::Rect,
        transform: &Affine,
        fill: Option<Color>,
        stroke: Option<Color>,
        stroke_width: f64,
        corner_radius: f64,
    ) {
        ops.push(Op::SaveGraphicsState);

        if let Some(fill_color) = fill {
            ops.push(Op::SetFillColor {
                col: vello_to_pdf_color(fill_color),
            });
        }
        if let Some(stroke_color) = stroke {
            ops.push(Op::SetOutlineColor {
                col: vello_to_pdf_color(stroke_color),
            });
            ops.push(Op::SetOutlineThickness {
                pt: Pt(stroke_width as f32),
            });
        }

        let mode = match (fill.is_some(), stroke.is_some()) {
            (true, true) => printpdf::PaintMode::FillStroke,
            (true, false) => printpdf::PaintMode::Fill,
            (false, true) => printpdf::PaintMode::Stroke,
            (false, false) => printpdf::PaintMode::Fill,
        };

        // Use rounded rect path if corner_radius > 0, otherwise simple polygon
        let points = if corner_radius > 0.0 {
            // Create a rounded rectangle using kurbo and convert to line points
            let rounded = kurbo::RoundedRect::from_rect(*rect, corner_radius);
            let path = rounded.into_path(0.1); // tolerance for curve flattening
            bezpath_to_line_points(&path, transform, self.config.height)
        } else {
            // Simple rectangle - transform corners directly
            let p0 = transform_point(transform, kurbo::Point::new(rect.x0, rect.y0));
            let p1 = transform_point(transform, kurbo::Point::new(rect.x1, rect.y1));

            // Flip Y for PDF
            let y0 = self.config.height - p0.y;
            let y1 = self.config.height - p1.y;

            vec![
                LinePoint {
                    p: PdfPoint {
                        x: Pt(p0.x as f32),
                        y: Pt(y0 as f32),
                    },
                    bezier: false,
                },
                LinePoint {
                    p: PdfPoint {
                        x: Pt(p1.x as f32),
                        y: Pt(y0 as f32),
                    },
                    bezier: false,
                },
                LinePoint {
                    p: PdfPoint {
                        x: Pt(p1.x as f32),
                        y: Pt(y1 as f32),
                    },
                    bezier: false,
                },
                LinePoint {
                    p: PdfPoint {
                        x: Pt(p0.x as f32),
                        y: Pt(y1 as f32),
                    },
                    bezier: false,
                },
            ]
        };

        if !points.is_empty() {
            ops.push(Op::DrawPolygon {
                polygon: printpdf::Polygon {
                    rings: vec![PolygonRing { points }],
                    mode,
                    winding_order: printpdf::WindingOrder::NonZero,
                },
            });
        }

        ops.push(Op::RestoreGraphicsState);
    }

    /// Render a circle.
    fn render_circle(
        &self,
        ops: &mut Vec<Op>,
        center: kurbo::Point,
        radius: f64,
        fill: Option<Color>,
        stroke: Option<Color>,
        stroke_width: f64,
    ) {
        // Flip Y for PDF
        let cy = self.config.height - center.y;

        // Approximate circle with line segments
        let points = circle_to_line_points(center.x, cy, radius);

        ops.push(Op::SaveGraphicsState);

        if let Some(fill_color) = fill {
            ops.push(Op::SetFillColor {
                col: vello_to_pdf_color(fill_color),
            });
        }
        if let Some(stroke_color) = stroke {
            ops.push(Op::SetOutlineColor {
                col: vello_to_pdf_color(stroke_color),
            });
            ops.push(Op::SetOutlineThickness {
                pt: Pt(stroke_width as f32),
            });
        }

        let mode = match (fill.is_some(), stroke.is_some()) {
            (true, true) => printpdf::PaintMode::FillStroke,
            (true, false) => printpdf::PaintMode::Fill,
            (false, true) => printpdf::PaintMode::Stroke,
            (false, false) => printpdf::PaintMode::Fill,
        };

        ops.push(Op::DrawPolygon {
            polygon: printpdf::Polygon {
                rings: vec![PolygonRing { points }],
                mode,
                winding_order: printpdf::WindingOrder::NonZero,
            },
        });

        ops.push(Op::RestoreGraphicsState);
    }

    /// Render an ellipse.
    #[allow(clippy::too_many_arguments)]
    fn render_ellipse(
        &self,
        ops: &mut Vec<Op>,
        center: kurbo::Point,
        radius_x: f64,
        radius_y: f64,
        fill: Option<Color>,
        stroke: Option<Color>,
        stroke_width: f64,
    ) {
        // Flip Y for PDF
        let cy = self.config.height - center.y;

        // Approximate ellipse with line points
        let points = ellipse_to_line_points(center.x, cy, radius_x, radius_y);

        ops.push(Op::SaveGraphicsState);

        if let Some(fill_color) = fill {
            ops.push(Op::SetFillColor {
                col: vello_to_pdf_color(fill_color),
            });
        }
        if let Some(stroke_color) = stroke {
            ops.push(Op::SetOutlineColor {
                col: vello_to_pdf_color(stroke_color),
            });
            ops.push(Op::SetOutlineThickness {
                pt: Pt(stroke_width as f32),
            });
        }

        let mode = match (fill.is_some(), stroke.is_some()) {
            (true, true) => printpdf::PaintMode::FillStroke,
            (true, false) => printpdf::PaintMode::Fill,
            (false, true) => printpdf::PaintMode::Stroke,
            (false, false) => printpdf::PaintMode::Fill,
        };

        ops.push(Op::DrawPolygon {
            polygon: printpdf::Polygon {
                rings: vec![PolygonRing { points }],
                mode,
                winding_order: printpdf::WindingOrder::NonZero,
            },
        });

        ops.push(Op::RestoreGraphicsState);
    }
}

/// Convert a vello Color to printpdf Color.
fn vello_to_pdf_color(color: Color) -> PdfColor {
    let rgba = color.to_rgba8();
    PdfColor::Rgb(printpdf::Rgb {
        r: f32::from(rgba.r) / 255.0,
        g: f32::from(rgba.g) / 255.0,
        b: f32::from(rgba.b) / 255.0,
        icc_profile: None,
    })
}

/// Transform a point by an affine transform.
fn transform_point(transform: &Affine, point: kurbo::Point) -> kurbo::Point {
    *transform * point
}

/// Convert a bezier path to line points (flattening curves).
fn bezpath_to_line_points(
    path: &kurbo::BezPath,
    transform: &Affine,
    page_height: f64,
) -> Vec<LinePoint> {
    let mut points = Vec::new();
    let mut current_pos = kurbo::Point::ZERO;

    for el in path.elements() {
        match el {
            kurbo::PathEl::MoveTo(p) => {
                let pt = transform_point(transform, *p);
                let y = page_height - pt.y;
                points.push(LinePoint {
                    p: PdfPoint {
                        x: Pt(pt.x as f32),
                        y: Pt(y as f32),
                    },
                    bezier: false,
                });
                current_pos = *p;
            }
            kurbo::PathEl::LineTo(p) => {
                let pt = transform_point(transform, *p);
                let y = page_height - pt.y;
                points.push(LinePoint {
                    p: PdfPoint {
                        x: Pt(pt.x as f32),
                        y: Pt(y as f32),
                    },
                    bezier: false,
                });
                current_pos = *p;
            }
            kurbo::PathEl::QuadTo(p1, p2) => {
                // Flatten quadratic bezier with line segments
                let steps = 8;
                for i in 1..=steps {
                    let t = i as f64 / steps as f64;
                    let p = quad_bezier_point(current_pos, *p1, *p2, t);
                    let pt = transform_point(transform, p);
                    let y = page_height - pt.y;
                    points.push(LinePoint {
                        p: PdfPoint {
                            x: Pt(pt.x as f32),
                            y: Pt(y as f32),
                        },
                        bezier: false,
                    });
                }
                current_pos = *p2;
            }
            kurbo::PathEl::CurveTo(p1, p2, p3) => {
                // Flatten cubic bezier with line segments
                let steps = 8;
                for i in 1..=steps {
                    let t = i as f64 / steps as f64;
                    let p = cubic_bezier_point(current_pos, *p1, *p2, *p3, t);
                    let pt = transform_point(transform, p);
                    let y = page_height - pt.y;
                    points.push(LinePoint {
                        p: PdfPoint {
                            x: Pt(pt.x as f32),
                            y: Pt(y as f32),
                        },
                        bezier: false,
                    });
                }
                current_pos = *p3;
            }
            kurbo::PathEl::ClosePath => {
                // Path will be closed by polygon
            }
        }
    }

    points
}

/// Evaluate quadratic bezier at parameter t.
fn quad_bezier_point(p0: kurbo::Point, p1: kurbo::Point, p2: kurbo::Point, t: f64) -> kurbo::Point {
    let mt = 1.0 - t;
    kurbo::Point::new(
        mt * mt * p0.x + 2.0 * mt * t * p1.x + t * t * p2.x,
        mt * mt * p0.y + 2.0 * mt * t * p1.y + t * t * p2.y,
    )
}

/// Evaluate cubic bezier at parameter t.
fn cubic_bezier_point(
    p0: kurbo::Point,
    p1: kurbo::Point,
    p2: kurbo::Point,
    p3: kurbo::Point,
    t: f64,
) -> kurbo::Point {
    let mt = 1.0 - t;
    let mt2 = mt * mt;
    let t2 = t * t;
    kurbo::Point::new(
        mt2 * mt * p0.x + 3.0 * mt2 * t * p1.x + 3.0 * mt * t2 * p2.x + t2 * t * p3.x,
        mt2 * mt * p0.y + 3.0 * mt2 * t * p1.y + 3.0 * mt * t2 * p2.y + t2 * t * p3.y,
    )
}

/// Approximate a circle with line points.
fn circle_to_line_points(cx: f64, cy: f64, r: f64) -> Vec<LinePoint> {
    let segments = 32;
    let mut points = Vec::with_capacity(segments);

    for i in 0..segments {
        let angle = 2.0 * std::f64::consts::PI * (i as f64) / (segments as f64);
        let x = cx + r * angle.cos();
        let y = cy + r * angle.sin();
        points.push(LinePoint {
            p: PdfPoint {
                x: Pt(x as f32),
                y: Pt(y as f32),
            },
            bezier: false,
        });
    }

    points
}

/// Approximate an ellipse with line points.
fn ellipse_to_line_points(cx: f64, cy: f64, rx: f64, ry: f64) -> Vec<LinePoint> {
    let segments = 32;
    let mut points = Vec::with_capacity(segments);

    for i in 0..segments {
        let angle = 2.0 * std::f64::consts::PI * (i as f64) / (segments as f64);
        let x = cx + rx * angle.cos();
        let y = cy + ry * angle.sin();
        points.push(LinePoint {
            p: PdfPoint {
                x: Pt(x as f32),
                y: Pt(y as f32),
            },
            bezier: false,
        });
    }

    points
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engraver::scene::id::{ElementType, SemanticId};
    use kurbo::{Point, Rect};
    use peniko::Color;

    #[test]
    fn test_default_config() {
        let config = PdfExportConfig::default();
        assert_eq!(config.width, 612.0);
        assert_eq!(config.height, 792.0);
    }

    #[test]
    fn test_a4_config() {
        let config = PdfExportConfig::a4();
        assert_eq!(config.width, 595.0);
        assert_eq!(config.height, 842.0);
    }

    #[test]
    fn test_config_builder() {
        let config = PdfExportConfig::default()
            .with_title("Test Document")
            .with_author("Test Author")
            .with_background(Color::WHITE);

        assert_eq!(config.title, "Test Document");
        assert_eq!(config.author, Some("Test Author".to_string()));
        assert!(config.background.is_some());
    }

    #[test]
    fn test_empty_scene_serialize() {
        let config = PdfExportConfig::default();
        let mut serializer = PdfSerializer::new(config);

        let scene = SceneNode::group(SemanticId::page(1));
        let result = serializer.serialize(&scene);

        assert!(result.is_ok());
        let bytes = result.unwrap();
        assert!(!bytes.is_empty());
        // PDF files should start with %PDF
        assert!(bytes.starts_with(b"%PDF"));
    }

    #[test]
    fn test_scene_with_rect() {
        let config = PdfExportConfig::default();
        let mut serializer = PdfSerializer::new(config);

        let scene = SceneNode::anonymous_leaf(vec![PaintCommand::filled_rect(
            Rect::new(10.0, 20.0, 100.0, 80.0),
            Color::BLACK,
        )]);

        let result = serializer.serialize(&scene);
        assert!(result.is_ok());
        let bytes = result.unwrap();
        assert!(bytes.starts_with(b"%PDF"));
    }

    #[test]
    fn test_scene_with_line() {
        let config = PdfExportConfig::default();
        let mut serializer = PdfSerializer::new(config);

        let scene = SceneNode::anonymous_leaf(vec![PaintCommand::Line {
            start: Point::new(0.0, 0.0),
            end: Point::new(100.0, 100.0),
            width: 1.0,
            color: Color::BLACK,
            line_cap: LineCap::Round,
        }]);

        let result = serializer.serialize(&scene);
        assert!(result.is_ok());
    }

    #[test]
    fn test_invisible_node_skipped() {
        let config = PdfExportConfig::default();
        let mut serializer = PdfSerializer::new(config);

        let mut scene = SceneNode::group(SemanticId::page(1));
        let mut invisible = SceneNode::leaf(
            SemanticId::new(ElementType::Note, 1),
            vec![PaintCommand::filled_rect(
                Rect::new(0.0, 0.0, 10.0, 10.0),
                Color::BLACK,
            )],
        );
        invisible.visible = false;
        scene.add_child(invisible);

        let result = serializer.serialize(&scene);
        assert!(result.is_ok());
    }
}
