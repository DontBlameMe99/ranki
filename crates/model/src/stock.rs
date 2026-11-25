//! Anki-compatible stock note types used by the CLI and storage.

use crate::CardTemplate;
use crate::NoteKind;
use crate::NoteModel;

/// Anki's `originalStockKind` value for Basic.
pub const ORIGINAL_STOCK_KIND_BASIC: i32 = 1;
/// Anki's `originalStockKind` value for Basic (and reversed card).
pub const ORIGINAL_STOCK_KIND_BASIC_AND_REVERSED: i32 = 2;
/// Anki's `originalStockKind` value for Cloze.
pub const ORIGINAL_STOCK_KIND_CLOZE: i32 = 5;
/// Anki's `originalStockKind` value for Image Occlusion.
pub const ORIGINAL_STOCK_KIND_IMAGE_OCCLUSION: i32 = 6;

const NORMAL_CSS: &str = ".card {\n    font-family: arial;\n    font-size: 20px;\n    text-align: center;\n    color: black;\n    background-color: white;\n}\n";
const CLOZE_CSS: &str = ".cloze {\n    font-weight: bold;\n    color: blue;\n}\n.nightMode .cloze {\n    color: lightblue;\n}\n";
const IMAGE_OCCLUSION_CSS: &str = "#image-occlusion-canvas {\n    --inactive-shape-color: #ffeba2;\n    --active-shape-color: #ff8e8e;\n    --inactive-shape-border: 1px #212121;\n    --active-shape-border: 1px #212121;\n    --highlight-shape-color: #ff8e8e00;\n    --highlight-shape-border: 1px #ff8e8e;\n}\n\n.card {\n    font-family: arial;\n    font-size: 20px;\n    text-align: center;\n    color: black;\n    background-color: white;\n}\n";
const IMAGE_OCCLUSION_QFMT: &str = "{{#Header}}<div>{{Header}}</div>{{/Header}}\n<div style=\"display: none\">{{cloze:Occlusion}}</div>\n<div id=\"err\"></div>\n<div id=\"image-occlusion-container\">\n    {{Image}}\n    <canvas id=\"image-occlusion-canvas\"></canvas>\n</div>\n<script>\ntry { anki.imageOcclusion.setup(); }\ncatch (exc) { document.getElementById(\"err\").innerHTML = `<error><br><br>${exc}`; }\n</script>\n";
const IMAGE_OCCLUSION_AFMT: &str = "{{#Header}}<div>{{Header}}</div>{{/Header}}\n<div style=\"display: none\">{{cloze:Occlusion}}</div>\n<div id=\"err\"></div>\n<div id=\"image-occlusion-container\">\n    {{Image}}\n    <canvas id=\"image-occlusion-canvas\"></canvas>\n</div>\n<script>\ntry { anki.imageOcclusion.setup(); }\ncatch (exc) { document.getElementById(\"err\").innerHTML = `<error><br><br>${exc}`; }\n</script>\n\n<div><button id=\"toggle\">Toggle masks</button></div>\n{{#Back Extra}}<div>{{Back Extra}}</div>{{/Back Extra}}\n";

/// Stock note types Ranki can install into a collection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StockModelKind {
    Basic,
    BasicAndReversed,
    Cloze,
    ImageOcclusion,
}

impl StockModelKind {
    /// Every stock type, in Anki's new-collection order.
    pub const ALL: [Self; 4] = [
        Self::Basic,
        Self::BasicAndReversed,
        Self::Cloze,
        Self::ImageOcclusion,
    ];

    /// Name the stock type is installed under.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Basic => "Basic",
            Self::BasicAndReversed => "Basic (and reversed card)",
            Self::Cloze => "Cloze",
            Self::ImageOcclusion => "Image Occlusion",
        }
    }

    /// Stock kind for a note type name, ignoring case.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.name().eq_ignore_ascii_case(name.trim()))
    }

    /// Stock kind of a stored model, trusting Anki's `originalStockKind` first.
    #[must_use]
    pub fn of(model: &NoteModel) -> Option<Self> {
        match model.original_stock_kind {
            Some(ORIGINAL_STOCK_KIND_BASIC) => Some(Self::Basic),
            Some(ORIGINAL_STOCK_KIND_BASIC_AND_REVERSED) => {
                Some(Self::BasicAndReversed)
            }
            Some(ORIGINAL_STOCK_KIND_CLOZE) => Some(Self::Cloze),
            Some(ORIGINAL_STOCK_KIND_IMAGE_OCCLUSION) => {
                Some(Self::ImageOcclusion)
            }
            _ => Self::from_name(&model.name),
        }
    }

    /// Definition installed into a collection. The id is a placeholder.
    #[must_use]
    pub fn definition(self) -> NoteModel {
        match self {
            Self::Basic => basic(),
            Self::BasicAndReversed => basic_and_reversed(),
            Self::Cloze => cloze(),
            Self::ImageOcclusion => image_occlusion(),
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "the four template fields belong together"
)]
fn template(
    ord: u16,
    name: &str,
    question: &str,
    answer: &str,
) -> CardTemplate {
    CardTemplate {
        name: name.to_owned(),
        question_format: question.to_owned(),
        answer_format: answer.to_owned(),
        ord,
    }
}

fn basic() -> NoteModel {
    NoteModel {
        id: 0,
        name: "Basic".to_owned(),
        fields: vec!["Front".to_owned(), "Back".to_owned()],
        templates: vec![template(
            0,
            "Card 1",
            "{{Front}}",
            "{{FrontSide}}\n\n<hr id=answer>\n\n{{Back}}",
        )],
        css: NORMAL_CSS.to_owned(),
        sort_field_index: 0,
        kind: NoteKind::Normal,
        original_stock_kind: Some(ORIGINAL_STOCK_KIND_BASIC),
        field_tags: vec![None, None],
        template_ordinals: vec![0],
        raw_config: None,
    }
}

fn basic_and_reversed() -> NoteModel {
    let mut reversed = basic();
    "Basic (and reversed card)".clone_into(&mut reversed.name);
    reversed.original_stock_kind = Some(ORIGINAL_STOCK_KIND_BASIC_AND_REVERSED);
    reversed.templates.push(template(
        1,
        "Card 2",
        "{{Back}}",
        "{{FrontSide}}\n\n<hr id=answer>\n\n{{Front}}",
    ));
    reversed.template_ordinals.push(1);
    reversed
}

fn cloze() -> NoteModel {
    NoteModel {
        id: 0,
        name: "Cloze".to_owned(),
        fields: vec!["Text".to_owned(), "Back Extra".to_owned()],
        templates: vec![template(
            0,
            "Cloze",
            "{{cloze:Text}}",
            "{{cloze:Text}}<br>\n{{Back Extra}}",
        )],
        css: format!("{NORMAL_CSS}{CLOZE_CSS}"),
        sort_field_index: 0,
        kind: NoteKind::Cloze,
        original_stock_kind: Some(ORIGINAL_STOCK_KIND_CLOZE),
        field_tags: vec![Some(0), Some(1)],
        template_ordinals: vec![0],
        raw_config: None,
    }
}

fn image_occlusion() -> NoteModel {
    NoteModel {
        id: 0,
        name: "Image Occlusion".to_owned(),
        fields: vec![
            "Occlusion".to_owned(),
            "Image".to_owned(),
            "Header".to_owned(),
            "Back Extra".to_owned(),
            "Comments".to_owned(),
        ],
        templates: vec![template(
            0,
            "Image Occlusion",
            IMAGE_OCCLUSION_QFMT,
            IMAGE_OCCLUSION_AFMT,
        )],
        css: IMAGE_OCCLUSION_CSS.to_owned(),
        sort_field_index: 0,
        kind: NoteKind::Cloze,
        original_stock_kind: Some(ORIGINAL_STOCK_KIND_IMAGE_OCCLUSION),
        field_tags: vec![Some(0), Some(1), Some(2), Some(3), Some(4)],
        template_ordinals: vec![0],
        raw_config: None,
    }
}

/// Stock note types available to Ranki, with placeholder ids.
///
/// Ids are assigned by the collection when a stock type is installed.
#[must_use]
pub fn stock_models() -> Vec<NoteModel> {
    StockModelKind::ALL.map(StockModelKind::definition).to_vec()
}

#[cfg(test)]
mod tests {
    use super::ORIGINAL_STOCK_KIND_IMAGE_OCCLUSION;
    use super::StockModelKind;
    use super::stock_models;
    use crate::NoteKind;

    #[test]
    fn stock_models_match_anki_templates_and_ordinals() {
        let models = stock_models();
        assert_eq!(models.len(), 4);
        assert_eq!(models[0].templates.len(), 1);
        assert_eq!(models[1].templates.len(), 2);
        assert_eq!(
            models[1]
                .templates
                .iter()
                .map(|item| item.ord)
                .collect::<Vec<_>>(),
            [0, 1]
        );
        assert_eq!(models[2].kind, NoteKind::Cloze);
        assert_eq!(models[2].field_tags, [Some(0), Some(1)]);
        assert_eq!(
            models[3].original_stock_kind,
            Some(ORIGINAL_STOCK_KIND_IMAGE_OCCLUSION)
        );
        assert_eq!(models[3].field_tags.len(), 5);
        assert!(
            models[3].templates[0]
                .question_format
                .contains("{{cloze:Occlusion}}")
        );
    }

    #[test]
    fn stock_kinds_map_to_their_names_and_definitions() {
        for kind in StockModelKind::ALL {
            assert_eq!(StockModelKind::from_name(kind.name()), Some(kind));
            let definition = kind.definition();
            assert_eq!(definition.name, kind.name());
            assert_eq!(StockModelKind::of(&definition), Some(kind));
        }
        assert_eq!(
            StockModelKind::from_name("basic (AND reversed card)"),
            Some(StockModelKind::BasicAndReversed)
        );
        assert_eq!(StockModelKind::from_name("Custom"), None);
        let mut renamed = StockModelKind::Cloze.definition();
        renamed.name = String::from("Meine Karten");
        assert_eq!(StockModelKind::of(&renamed), Some(StockModelKind::Cloze));
    }
}
