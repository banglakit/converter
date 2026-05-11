/**
 * Entry point for Docs conversion — called from the menu.
 */
function showConvertDialogDocs() {
  var doc = DocumentApp.getActiveDocument();
  var selection = doc.getSelection();
  var runs;
  var scope;

  if (selection) {
    runs = extractRunsFromSelection_(selection);
    scope = 'selection';
  } else {
    runs = extractRunsFromBody_(doc.getBody());
    scope = 'document';
  }

  showConvertDialog_(runs, scope, 'docs');
}

/**
 * Extracts {id, text, font} runs from the entire document body.
 * Segments each paragraph into contiguous runs of the same font family.
 *
 * ID format: "docs:P<paraIndex>:C<charStart>-<charEnd>"
 */
function extractRunsFromBody_(body) {
  var runs = [];
  var numChildren = body.getNumChildren();

  for (var i = 0; i < numChildren; i++) {
    var child = body.getChild(i);
    if (child.getType() === DocumentApp.ElementType.PARAGRAPH ||
        child.getType() === DocumentApp.ElementType.LIST_ITEM) {
      var textEl = child.editAsText();
      var paraRuns = segmentByFont_(textEl, i);
      runs = runs.concat(paraRuns);
    }
  }

  return runs;
}

/**
 * Extracts runs from the current selection only.
 */
function extractRunsFromSelection_(selection) {
  var runs = [];
  var elements = selection.getRangeElements();

  for (var i = 0; i < elements.length; i++) {
    var rangeEl = elements[i];
    var el = rangeEl.getElement();

    if (el.getType() === DocumentApp.ElementType.TEXT) {
      var textEl = el.editAsText();
      var start = rangeEl.isPartial() ? rangeEl.getStartOffset() : 0;
      var end = rangeEl.isPartial() ? rangeEl.getEndOffsetInclusive() : textEl.getText().length - 1;

      // Find the paragraph index for the ID
      var para = el.getParent();
      var paraIndex = para.getParent().getChildIndex(para);

      var segmented = segmentByFont_(textEl, paraIndex, start, end);
      runs = runs.concat(segmented);
    }
  }

  return runs;
}

/**
 * Segments a Text element into contiguous runs of the same font family.
 *
 * @param {GoogleAppsScript.Document.Text} textEl - The text element to segment.
 * @param {number} paraIndex - Paragraph index for building run IDs.
 * @param {number} [startOffset=0] - Start character offset (inclusive).
 * @param {number} [endOffset] - End character offset (inclusive). Defaults to end of text.
 * @returns {Array<{id: string, text: string, font: string|null}>}
 */
function segmentByFont_(textEl, paraIndex, startOffset, endOffset) {
  var fullText = textEl.getText();
  if (!fullText) return [];

  var start = (startOffset !== undefined) ? startOffset : 0;
  var end = (endOffset !== undefined) ? endOffset : fullText.length - 1;
  if (start > end) return [];

  var runs = [];
  var runStart = start;
  var currentFont = textEl.getFontFamily(start);

  for (var i = start + 1; i <= end + 1; i++) {
    var font = (i <= end) ? textEl.getFontFamily(i) : null;
    if (font !== currentFont || i > end) {
      var runText = fullText.substring(runStart, i);
      if (runText.length > 0) {
        runs.push({
          id: 'docs:P' + paraIndex + ':C' + runStart + '-' + (i - 1),
          text: runText,
          font: currentFont
        });
      }
      runStart = i;
      currentFont = font;
    }
  }

  return runs;
}
