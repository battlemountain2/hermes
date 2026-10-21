use super::*;
use std::{
    io::{Cursor, Write},
    os::fd::AsRawFd,
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);

struct Fixture(std::path::PathBuf);

impl Fixture {
    fn new(members: &[(&str, &str)]) -> Self {
        Self::compressed(members, zip::CompressionMethod::Stored)
    }

    fn compressed(members: &[(&str, &str)], method: zip::CompressionMethod) -> Self {
        let path = std::env::temp_dir().join(format!(
            "hermes-office-{}-{}.zip",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, xml) in members {
            archive
                .start_file(
                    *name,
                    zip::write::SimpleFileOptions::default().compression_method(method),
                )
                .expect("member");
            archive.write_all(xml.as_bytes()).expect("member XML");
        }
        let bytes = archive.finish().expect("ZIP").into_inner();
        let mut file = File::options()
            .write(true)
            .create_new(true)
            .open(&path)
            .expect("new fixture");
        file.write_all(&bytes).expect("fixture bytes");
        Self(path)
    }

    fn text(&self, limit: usize) -> Result<String, String> {
        String::from_utf8(extract_text(&self.0, limit)?).map_err(|error| error.to_string())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_file(&self.0).expect("remove fixture");
    }
}

#[test]
fn docx_runs_paragraphs_breaks_and_tables_keep_their_boundaries() {
    let fixture = Fixture::new(&[(
        "word/document.xml",
        r#"<w:document xmlns:w="word"><w:body><w:p><w:r><w:t>Hel</w:t></w:r><w:r><w:t>lo &amp; &#x1f980;</w:t><w:tab/><w:t>tab</w:t><w:br/><w:t>next</w:t></w:r></w:p><w:tbl><w:tr><w:tc><w:p><w:t>Left</w:t></w:p></w:tc><w:tc><w:p><w:t>Right</w:t></w:p></w:tc></w:tr><w:tr><w:tc><w:p><w:t>A</w:t></w:p><w:p><w:t>B</w:t></w:p></w:tc><w:tc><w:p><w:t>C</w:t></w:p></w:tc></w:tr></w:tbl><w:p><w:instrText>ignored field</w:instrText><w:delText>deleted</w:delText><w:t>&amp;lt;</w:t></w:p></w:body></w:document>"#,
    )]);
    assert_eq!(
        fixture.text(4096).expect("DOCX text"),
        "Hello & 🦀\ttab\nnext\nLeft\tRight\nA\nB\tC\n&lt;"
    );
}

#[test]
fn anonymous_descriptors_and_misleading_names_use_package_structure() {
    let fixture = Fixture::new(&[(
        "word/document.xml",
        "<document><p><t>Real content</t></p></document>",
    )]);
    let file = File::open(&fixture.0).expect("fixture");
    let descriptor = format!("/proc/self/fd/{}", file.as_raw_fd());
    assert_eq!(
        extract_text(Path::new(&descriptor), 100).expect("descriptor"),
        b"Real content"
    );
    assert_eq!(fixture.text(100).expect("ZIP name"), "Real content");
}

#[test]
fn odf_mixed_spans_spaces_and_tables_are_readable() {
    let fixture = Fixture::new(&[(
        "content.xml",
        r#"<office:document xmlns:office="office" xmlns:text="text" xmlns:table="table"><text:h>Heading</text:h><text:p>A<text:span>B</text:span><text:s text:c="3"/>C<text:tab/>D<text:line-break/>E</text:p><table:table-row><table:table-cell><text:p>one</text:p></table:table-cell><table:table-cell><text:p>two</text:p></table:table-cell></table:table-row></office:document>"#,
    )]);
    assert_eq!(
        fixture.text(1024).expect("ODF text"),
        "Heading\nAB   C\tD\nE\none\ttwo"
    );
}

fn presentation<'a>(slide_list: &'a str, relationships: &'a str) -> Vec<(&'a str, &'a str)> {
    vec![
        ("ppt/presentation.xml", slide_list),
        ("ppt/_rels/presentation.xml.rels", relationships),
        (
            "ppt/slides/slide2.xml",
            "<sld><p><t>Second slide</t></p></sld>",
        ),
        (
            "ppt/slides/slide10.xml",
            "<sld><p><t>First</t><t> slide</t></p><p><t>Body</t></p></sld>",
        ),
        (
            "ppt/slides/slide1.xml",
            "<sld><p><t>Unreferenced</t></p></sld>",
        ),
        (
            "ppt/notesSlides/notesSlide1.xml",
            "<notes><p><t>Notes omitted</t></p></notes>",
        ),
    ]
}

const SLIDE_RELS: &str = r#"<Relationships><Relationship Id="second" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide2.xml"/><Relationship Id="first" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="/ppt/slides/slide10.xml"/></Relationships>"#;

#[test]
fn pptx_uses_presentation_relationship_order_not_zip_or_filename_order() {
    let fixture = Fixture::new(&presentation(
        r#"<p:presentation xmlns:p="p" xmlns:r="r"><p:sldIdLst><p:sldId id="400" r:id="first"/><p:sldId id="300" r:id="second"/></p:sldIdLst></p:presentation>"#,
        SLIDE_RELS,
    ));
    assert_eq!(
        fixture.text(4096).expect("slides"),
        "Slide 1\nFirst slide\nBody\n\nSlide 2\nSecond slide"
    );
}

#[test]
fn slide_limit_reports_omissions_without_accumulating_unbounded_paths() {
    let list = format!(
        "<presentation>{}</presentation>",
        "<sldId r:id=\"second\"/>".repeat(SLIDE_LIMIT + 1)
    );
    let fixture = Fixture::new(&presentation(&list, SLIDE_RELS));
    let text = fixture.text(100_000).expect("bounded deck");
    assert!(text.contains("Slide 256\nSecond slide"));
    assert!(!text.contains("Slide 257"));
    assert!(text.ends_with("…additional slides omitted"));
}

#[test]
fn missing_or_unsafe_slide_relationships_fail_without_fallback_text() {
    for relationships in [
        "<Relationships/>",
        "<Relationships><Relationship Id=\"first\" Type=\"internal/slide\" Target=\"../../../outside.xml\"/></Relationships>",
        "<Relationships><Relationship Id=\"first\" Type=\"internal/slide\" Target=\"slides/missing.xml\"/></Relationships>",
    ] {
        let fixture = Fixture::new(&presentation(
            "<presentation><sldId r:id=\"first\"/></presentation>",
            relationships,
        ));
        assert!(fixture.text(1000).is_err());
    }
}

#[test]
fn text_limits_preserve_utf8_and_never_exceed_the_requested_bytes() {
    let fixture = Fixture::new(&[(
        "word/document.xml",
        "<document><p><t>é🦀tail</t></p></document>",
    )]);
    for limit in 0..12 {
        let text = fixture.text(limit).expect("bounded text");
        assert!(text.len() <= limit);
        assert!("é🦀tail".starts_with(&text));
    }
}

#[test]
fn malformed_xml_dtd_custom_entities_and_excessive_nesting_are_rejected() {
    for xml in [
        "<document><p><t>broken</p></document>".to_string(),
        "<document><p><t>unfinished".to_string(),
        "<!DOCTYPE document [<!ENTITY foo SYSTEM 'file:///secret'>]><document><p><t>&foo;</t></p></document>".to_string(),
        "<document><p><t>&unknown;</t></p></document>".to_string(),
        format!("{}{}", "<p>".repeat(257), "</p>".repeat(257)),
    ] {
        let fixture = Fixture::new(&[("word/document.xml", &xml)]);
        assert!(fixture.text(1024).is_err());
    }
}

#[test]
fn external_relationships_and_odf_references_are_rejected() {
    for relationship in [
        "<Relationship TargetMode=\"External\" Target=\"relative.xml\"/>",
        "<Relationship Target=\"https://example.invalid/resource\"/>",
        "<Relationship Target=\"file:///secret\"/>",
        "<Relationship Target=\"//host/resource\"/>",
    ] {
        let fixture = Fixture::new(&[
            (
                "word/document.xml",
                "<document><p><t>hello</t></p></document>",
            ),
            ("word/_rels/document.xml.rels", relationship),
        ]);
        assert!(fixture.text(100).is_err());
    }
    for xml in [
        "<p><image xlink:href=\"https://example.invalid/image\"/></p>",
        "<p><object xlink:href=\"../outside\">link</object></p>",
    ] {
        let fixture = Fixture::new(&[("content.xml", xml)]);
        assert!(fixture.text(100).is_err());
    }
}

#[test]
fn docx_external_hyperlinks_preserve_visible_text_without_resolving_the_target() {
    let external = Fixture::new(&[(
        "word/document.xml",
        "<document><p><t>EXTERNAL CONTENT MUST NOT APPEAR</t></p></document>",
    )]);
    let target = format!("file://{}", external.0.display());
    let relationships = format!(
        r#"<Relationships><Relationship Id="link" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" TargetMode="External" Target="{target}"/><Relationship Id="image" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" TargetMode="External" Target="{target}"/><Relationship Id="web" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" TargetMode="External" Target="https://example.invalid/not-fetched"/></Relationships>"#
    );
    let fixture = Fixture::new(&[
        (
            "word/document.xml",
            r#"<w:document xmlns:w="word" xmlns:r="relationships"><w:p><w:r><w:t>Visit </w:t></w:r><w:hyperlink r:id="link"><w:r><w:t>the link</w:t></w:r></w:hyperlink><w:r><w:t> for details.</w:t></w:r></w:p><w:p><w:r><w:drawing r:link="image"/><w:t>Image caption</w:t></w:r></w:p></w:document>"#,
        ),
        ("word/_rels/document.xml.rels", &relationships),
    ]);
    let expected = "Visit the link for details.\nImage caption";
    assert_eq!(fixture.text(1024).expect("hyperlinked text"), expected);
    drop(external);
    assert_eq!(
        fixture
            .text(1024)
            .expect("missing external target is irrelevant"),
        expected
    );
}

#[test]
fn odf_hyperlinks_preserve_labels_without_reading_external_targets() {
    for target in [
        "https://example.invalid/no-fetch",
        "file:///private/no-fetch",
        "../outside",
        "#bookmark",
    ] {
        let xml = format!(
            "<office:document xmlns:office=\"office\" xmlns:text=\"text\" xmlns:xlink=\"xlink\"><text:p>Read <text:a xlink:href=\"{target}\">this label</text:a> here.</text:p></office:document>"
        );
        let fixture = Fixture::new(&[("content.xml", &xml)]);
        assert_eq!(fixture.text(100).unwrap(), "Read this label here.");
        Package::open(File::open(&fixture.0).unwrap())
            .unwrap()
            .validate_spreadsheet_xml()
            .unwrap();
    }
}

#[test]
fn worksheet_bounds_include_implicit_cells_and_nonstandard_part_names() {
    for name in ["xl/worksheets/sheet1.xml", "parts/relocated.part"] {
        let fixture = Fixture::new(&[
            (
                "_rels/.rels",
                "<Relationships><Relationship Id=\"sheet\" Type=\"internal/worksheet\" Target=\"parts/relocated.part\"/></Relationships>",
            ),
            (
                name,
                "<worksheet><sheetData><row><c><v>1</v></c></row><row><c r=\"XFD1048576\"><v>2</v></c></row></sheetData></worksheet>",
            ),
        ]);
        let error = Package::open(File::open(&fixture.0).unwrap())
            .unwrap()
            .validate_spreadsheet_xml()
            .unwrap_err();
        assert!(error.contains("allocation budget"), "{error}");
    }
    let fixture = Fixture::new(&[(
        "data.xml",
        "<worksheet><sheetData><row r=\"1048576\"/><row><c><v>1</v></c></row></sheetData></worksheet>",
    )]);
    assert!(
        Package::open(File::open(&fixture.0).unwrap())
            .unwrap()
            .validate_spreadsheet_xml()
            .is_err()
    );
}

#[test]
fn externally_targeted_slide_parts_are_rejected_even_with_internal_looking_paths() {
    for target in [
        "slides/slide10.xml",
        "https://example.invalid/slide.xml",
        "file:///outside.xml",
    ] {
        let relationships = format!(
            r#"<Relationships><Relationship Id="first" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" TargetMode="External" Target="{target}"/></Relationships>"#
        );
        let fixture = Fixture::new(&presentation(
            "<presentation><sldId r:id=\"first\"/></presentation>",
            &relationships,
        ));
        assert!(
            fixture
                .text(1024)
                .expect_err("external slide")
                .contains("External")
        );
    }
}

#[test]
fn unsafe_paths_expansion_ratios_and_oversized_members_are_rejected() {
    let fixture = Fixture::new(&[("../word/document.xml", "<document/>")]);
    assert!(fixture.text(100).is_err());
    let repeated = "a".repeat(1024 * 1024);
    let fixture = Fixture::compressed(
        &[("word/document.xml", &repeated)],
        zip::CompressionMethod::Deflated,
    );
    assert!(fixture.text(100).expect_err("ratio").contains("expansion"));
    let oversized = "a".repeat(MEMBER_LIMIT as usize + 1);
    let fixture = Fixture::new(&[("word/document.xml", &oversized)]);
    assert!(
        fixture
            .text(100)
            .expect_err("member bytes")
            .contains("expansion")
    );
}

#[test]
fn aggregate_expansion_and_entry_count_are_bounded() {
    let mut expanded = 0;
    for _ in 0..4 {
        check_expansion(MEMBER_LIMIT, MEMBER_LIMIT, &mut expanded)
            .expect("within aggregate budget");
    }
    assert!(check_expansion(1, 1, &mut expanded).is_err());
    let names: Vec<_> = (0..ENTRY_LIMIT + 1)
        .map(|index| format!("member{index}"))
        .collect();
    let members: Vec<_> = names.iter().map(|name| (name.as_str(), "")).collect();
    let fixture = Fixture::new(&members);
    assert!(
        fixture
            .text(100)
            .expect_err("member count")
            .contains("too many")
    );
}

#[test]
fn workbook_preflight_rejects_sparse_allocation_bombs_and_shared_string_counts() {
    for (name, xml) in [
        (
            "xl/worksheets/sheet1.xml",
            "<worksheet><c r=\"A1\"/><c r=\"XFD1048576\"/></worksheet>",
        ),
        ("xl/sharedStrings.xml", "<sst uniqueCount=\"4294967295\"/>"),
        (
            "xl/worksheets/sheet1.xml",
            "<!DOCTYPE worksheet><worksheet/>",
        ),
    ] {
        let fixture = Fixture::new(&[(name, xml)]);
        let mut package =
            Package::open(File::open(&fixture.0).expect("fixture")).expect("ZIP structure");
        assert!(package.validate_spreadsheet_xml().is_err());
    }
    assert_eq!(
        cell_position("XFD1048576").expect("last cell"),
        (1_048_576, 16_384)
    );
    for reference in [
        "",
        "A",
        "A0",
        "0",
        "XFE1",
        "A1048577",
        "A999999999999999999999999",
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAA1",
    ] {
        assert!(cell_position(reference).is_err());
    }
}

#[test]
fn unsupported_zip_structure_is_not_treated_as_a_document() {
    let fixture = Fixture::new(&[("unrelated.xml", "<p>Not Office</p>")]);
    assert!(fixture.text(100).is_err());
}
