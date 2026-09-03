// SPDX-License-Identifier: GPL-3.0-or-later

use super::office_xml_text;

#[test]
fn office_xml_is_reduced_to_readable_bounded_text() {
    let xml = r#"<w:document><w:p><w:r><w:t>Hello &amp; welcome</w:t></w:r></w:p><w:p><w:t>Hermes</w:t></w:p></w:document>"#;
    let text = office_xml_text(xml, 1024);
    assert!(text.contains("Hello & welcome"));
    assert!(text.contains("Hermes"));
    assert!(!text.contains("<w:"));

    assert!(office_xml_text(xml, 12).len() <= 12);
}
