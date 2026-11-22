//! Anki-compatible stock note types used by the CLI and storage.

use crate::CardTemplate;
use crate::NoteKind;
use crate::NoteModel;

/// Anki's `originalStockKind` value for Image Occlusion.
pub const ORIGINAL_STOCK_KIND_IMAGE_OCCLUSION: i32 = 6;

const NORMAL_CSS: &str = ".card {\n    font-family: arial;\n    font-size: 20px;\n    text-align: center;\n    color: black;\n    background-color: white;\n}\n";
const CLOZE_CSS: &str = ".cloze {\n    font-weight: bold;\n    color: blue;\n}\n.nightMode .cloze {\n    color: lightblue;\n}\n";
const IMAGE_OCCLUSION_CSS: &str = "#image-occlusion-canvas {\n    --inactive-shape-color: #ffeba2;\n    --active-shape-color: #ff8e8e;\n    --inactive-shape-border: 1px #212121;\n    --active-shape-border: 1px #212121;\n    --highlight-shape-color: #ff8e8e00;\n    --highlight-shape-border: 1px #ff8e8e;\n}\n\n.card {\n    font-family: arial;\n    font-size: 20px;\n    text-align: center;\n    color: black;\n    background-color: white;\n}\n";
const IMAGE_OCCLUSION_QFMT: &str = "{{#Header}}<div>{{Header}}</div>{{/Header}}\n<div style=\"display: none\">{{cloze:Occlusion}}</div>\n<div id=\"err\"></div>\n<div id=\"image-occlusion-container\">\n    {{Image}}\n    <canvas id=\"image-occlusion-canvas\"></canvas>\n</div>\n<script>\ntry { anki.imageOcclusion.setup(); }\ncatch (exc) { document.getElementById(\"err\").innerHTML = `<error><br><br>${exc}`; }\n</script>\n";
const IMAGE_OCCLUSION_AFMT: &str = "{{#Header}}<div>{{Header}}</div>{{/Header}}\n<div style=\"display: none\">{{cloze:Occlusion}}</div>\n<div id=\"err\"></div>\n<div id=\"image-occlusion-container\">\n    {{Image}}\n    <canvas id=\"image-occlusion-canvas\"></canvas>\n</div>\n<script>\ntry { anki.imageOcclusion.setup(); }\ncatch (exc) { document.getElementById(\"err\").innerHTML = `<error><br><br>${exc}`; }\n</script>\n\n<div><button id=\"toggle\">Toggle masks</button></div>\n{{#Back Extra}}<div>{{Back Extra}}</div>{{/Back Extra}}\n";

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
        kind: NoteKind::Normal,
        original_stock_kind: Some(1),
        field_tags: vec![None, None],
        template_ordinals: vec![0],
        raw_config: None,
    }
}

/// Stock note types available to Ranki, with placeholder ids.
///
/// Ids are assigned by the collection when a stock type is installed.
#[must_use]
pub fn stock_models() -> Vec<NoteModel> {
    let mut reversed = basic();
    "Basic (and reversed card)".clone_into(&mut reversed.name);
    reversed.original_stock_kind = Some(2);
    reversed.templates.push(template(
        1,
        "Card 2",
        "{{Back}}",
        "{{FrontSide}}\n\n<hr id=answer>\n\n{{Front}}",
    ));
    reversed.template_ordinals.push(1);

    let cloze = NoteModel {
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
        kind: NoteKind::Cloze,
        original_stock_kind: Some(5),
        field_tags: vec![Some(0), Some(1)],
        template_ordinals: vec![0],
        raw_config: None,
    };

    let occlusion = NoteModel {
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
        kind: NoteKind::Cloze,
        original_stock_kind: Some(ORIGINAL_STOCK_KIND_IMAGE_OCCLUSION),
        field_tags: vec![Some(0), Some(1), Some(2), Some(3), Some(4)],
        template_ordinals: vec![0],
        raw_config: None,
    };

    vec![basic(), reversed, cloze, occlusion]
}

#[cfg(test)]
mod tests {
    use super::ORIGINAL_STOCK_KIND_IMAGE_OCCLUSION;
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
}
