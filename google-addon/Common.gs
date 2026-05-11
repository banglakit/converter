/**
 * Opens the convert dialog, passing extracted runs as template data.
 *
 * @param {Array<{id: string, text: string, font: string|null}>} runs - Extracted runs.
 * @param {string} scope - 'document' or 'selection'.
 * @param {string} editor - 'docs', 'sheets', or 'slides'.
 */
function showConvertDialog_(runs, scope, editor) {
  var template = HtmlService.createTemplateFromFile('dialog');
  template.runs = JSON.stringify(runs);
  template.scope = scope;
  template.editor = editor;

  var html = template.evaluate()
    .setWidth(400)
    .setHeight(350)
    .setSandboxMode(HtmlService.SandboxMode.IFRAME);

  var ui;
  if (editor === 'docs') {
    ui = DocumentApp.getUi();
  } else if (editor === 'sheets') {
    ui = SpreadsheetApp.getUi();
  } else {
    ui = SlidesApp.getUi();
  }

  ui.showModalDialog(html, 'Banglakit Converter');
}

/**
 * Receives conversion results from the dialog and dispatches to the
 * correct editor's write-back function.
 *
 * Called from the dialog via google.script.run.applyConversions(results, editor).
 *
 * @param {Array<{id: string, newText: string, newFont: string}>} results
 * @param {string} editor - 'docs', 'sheets', or 'slides'
 */
function applyConversions(results, editor) {
  if (!results || results.length === 0) return;

  if (editor === 'docs') {
    applyConversionsDocs_(results);
  } else if (editor === 'sheets') {
    applyConversionsSheets_(results);
  } else if (editor === 'slides') {
    applyConversionsSlides_(results);
  }
}

/**
 * Applies conversion results to a Google Doc.
 * ID format: "docs:P<paraIndex>:C<charStart>-<charEnd>"
 */
function applyConversionsDocs_(results) {
  var doc = DocumentApp.getActiveDocument();
  var body = doc.getBody();

  // Process in reverse order so character offsets remain valid
  // (later offsets first, so earlier ones aren't shifted).
  var sorted = results.slice().sort(function(a, b) {
    var aParts = parseDocsId_(a.id);
    var bParts = parseDocsId_(b.id);
    if (bParts.para !== aParts.para) return bParts.para - aParts.para;
    return bParts.charStart - aParts.charStart;
  });

  for (var i = 0; i < sorted.length; i++) {
    var r = sorted[i];
    var parts = parseDocsId_(r.id);
    var child = body.getChild(parts.para);
    var textEl = child.editAsText();

    textEl.deleteText(parts.charStart, parts.charEnd);
    textEl.insertText(parts.charStart, r.newText);

    var newEnd = parts.charStart + r.newText.length - 1;
    textEl.setFontFamily(parts.charStart, newEnd, r.newFont);
  }
}

/**
 * Parses a Docs run ID into its components.
 * "docs:P5:C10-25" → {para: 5, charStart: 10, charEnd: 25}
 */
function parseDocsId_(id) {
  var match = id.match(/^docs:P(\d+):C(\d+)-(\d+)$/);
  return {
    para: parseInt(match[1], 10),
    charStart: parseInt(match[2], 10),
    charEnd: parseInt(match[3], 10)
  };
}

/**
 * Applies conversion results to a Google Sheet.
 * ID format: "sheets:R<row>:C<col>"
 */
function applyConversionsSheets_(results) {
  var sheet = SpreadsheetApp.getActiveSpreadsheet().getActiveSheet();

  for (var i = 0; i < results.length; i++) {
    var r = results[i];
    var match = r.id.match(/^sheets:R(\d+):C(\d+)$/);
    var row = parseInt(match[1], 10);
    var col = parseInt(match[2], 10);

    var cell = sheet.getRange(row, col);
    cell.setValue(r.newText);
    cell.setFontFamily(r.newFont);
  }
}

/**
 * Applies conversion results to a Google Slides presentation.
 * ID format: "slides:S<slideIdx>:SH<shapeIdx>:C<charStart>-<charEnd>"
 */
function applyConversionsSlides_(results) {
  var presentation = SlidesApp.getActivePresentation();
  var slides = presentation.getSlides();

  // Process in reverse character-offset order within each shape
  var sorted = results.slice().sort(function(a, b) {
    var aP = parseSlidesId_(a.id);
    var bP = parseSlidesId_(b.id);
    if (bP.slide !== aP.slide) return bP.slide - aP.slide;
    if (bP.shape !== aP.shape) return bP.shape - aP.shape;
    return bP.charStart - aP.charStart;
  });

  for (var i = 0; i < sorted.length; i++) {
    var r = sorted[i];
    var parts = parseSlidesId_(r.id);
    var shape = slides[parts.slide].getShapes()[parts.shape];
    var textRange = shape.getText();

    // Get the specific range and replace
    var range = textRange.getRange(parts.charStart, parts.charEnd + 1);
    var style = range.getTextStyle();

    range.setText(r.newText);

    // Re-acquire the range after setText (offsets may shift)
    var newRange = textRange.getRange(parts.charStart, parts.charStart + r.newText.length);
    newRange.getTextStyle().setFontFamily(r.newFont);
  }
}

/**
 * Parses a Slides run ID into its components.
 * "slides:S0:SH2:C10-25" → {slide: 0, shape: 2, charStart: 10, charEnd: 25}
 */
function parseSlidesId_(id) {
  var match = id.match(/^slides:S(\d+):SH(\d+):C(\d+)-(\d+)$/);
  return {
    slide: parseInt(match[1], 10),
    shape: parseInt(match[2], 10),
    charStart: parseInt(match[3], 10),
    charEnd: parseInt(match[4], 10)
  };
}
