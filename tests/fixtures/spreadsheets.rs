// SPDX-License-Identifier: GPL-3.0-or-later

use super::zip_util::create_zip;

/// Generates a valid ODS spreadsheet workbook with sample tabular financial data.
pub fn sample_ods_spreadsheet() -> Vec<u8> {
    let content_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
                         xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"
                         xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0">
  <office:body>
    <office:spreadsheet>
      <table:table table:name="Quarterly_Report">
        <table:table-row>
          <table:table-cell office:value-type="string"><text:p>Department</text:p></table:table-cell>
          <table:table-cell office:value-type="string"><text:p>Budget</text:p></table:table-cell>
          <table:table-cell office:value-type="string"><text:p>Actual</text:p></table:table-cell>
        </table:table-row>
        <table:table-row>
          <table:table-cell office:value-type="string"><text:p>Engineering</text:p></table:table-cell>
          <table:table-cell office:value-type="float" office:value="150000"><text:p>150000</text:p></table:table-cell>
          <table:table-cell office:value-type="float" office:value="142000"><text:p>142000</text:p></table:table-cell>
        </table:table-row>
        <table:table-row>
          <table:table-cell office:value-type="string"><text:p>Operations</text:p></table:table-cell>
          <table:table-cell office:value-type="float" office:value="85000"><text:p>85000</text:p></table:table-cell>
          <table:table-cell office:value-type="float" office:value="89000"><text:p>89000</text:p></table:table-cell>
        </table:table-row>
      </table:table>
    </office:spreadsheet>
  </office:body>
</office:document-content>"#;

    create_zip(&[
        ("mimetype", b"application/vnd.oasis.opendocument.spreadsheet"),
        ("content.xml", content_xml.as_bytes()),
    ])
}

/// Generates a valid XLSX spreadsheet workbook with sample sheet XML.
pub fn sample_xlsx_spreadsheet() -> Vec<u8> {
    let content_types = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/sharedStrings.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"/>
</Types>"#;

    let workbook_xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheets>
    <sheet name="Sheet1" sheetId="1" r:id="rId1" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"/>
  </sheets>
</workbook>"#;

    let shared_strings = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="4" uniqueCount="4">
  <si><t>Metric</t></si>
  <si><t>Q1</t></si>
  <si><t>Throughput</t></si>
  <si><t>99.9%</t></si>
</sst>"#;

    let sheet1_xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="1">
      <c r="A1" t="s"><v>0</v></c>
      <c r="B1" t="s"><v>1</v></c>
    </row>
    <row r="2">
      <c r="A2" t="s"><v>2</v></c>
      <c r="B2" t="s"><v>3</v></c>
    </row>
  </sheetData>
</worksheet>"#;

    create_zip(&[
        ("[Content_Types].xml", content_types.as_bytes()),
        ("xl/workbook.xml", workbook_xml.as_bytes()),
        ("xl/sharedStrings.xml", shared_strings.as_bytes()),
        ("xl/worksheets/sheet1.xml", sheet1_xml.as_bytes()),
    ])
}

/// Generates a binary XLS (Compound Document / BIFF8 signature).
pub fn sample_xls_spreadsheet() -> Vec<u8> {
    let mut out = Vec::new();
    // OLE Compound File signature: 0xD0 0xCF 0x11 0xE0 0xA1 0xB1 0x1A 0xE1
    out.extend_from_slice(&[0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1]);
    out.extend_from_slice(&[0x00; 504]); // 512-byte OLE header block
    out
}

pub fn sample_corrupted_spreadsheet() -> Vec<u8> {
    let mut data = sample_xlsx_spreadsheet();
    data.truncate(30);
    data
}
