/**
 * Entry point for Slides conversion.
 */
function showConvertDialogSlides() {
  var presentation = SlidesApp.getActivePresentation();
  var selection = presentation.getSelection();
  var runs;
  var scope;

  if (selection.getSelectionType() === SlidesApp.SelectionType.TEXT) {
    var textRange = selection.getTextRange();
    var pageElement = selection.getCurrentPage()
      ? null  // we'll get it from the text range's parent
      : null;
    // For text selection, extract from the selected text range
    runs = extractRunsFromTextRange_(textRange, 0, 0);
    scope = 'selection';
  } else {
    runs = extractRunsFromAllSlides_(presentation);
    scope = 'document';
  }

  showConvertDialog_(runs, scope, 'slides');
}

/**
 * Extracts runs from all slides in the presentation.
 */
function extractRunsFromAllSlides_(presentation) {
  var runs = [];
  var slides = presentation.getSlides();

  for (var s = 0; s < slides.length; s++) {
    var shapes = slides[s].getShapes();
    for (var sh = 0; sh < shapes.length; sh++) {
      var textRange;
      try {
        textRange = shapes[sh].getText();
      } catch (e) {
        continue;  // Shape has no text frame
      }
      var shapeRuns = extractRunsFromTextRange_(textRange, s, sh);
      runs = runs.concat(shapeRuns);
    }
  }

  return runs;
}

/**
 * Segments a Slides TextRange into contiguous runs of the same font.
 * ID format: "slides:S<slideIdx>:SH<shapeIdx>:C<charStart>-<charEnd>"
 */
function extractRunsFromTextRange_(textRange, slideIdx, shapeIdx) {
  var runs = [];
  var textRuns = textRange.getRuns();

  var offset = 0;
  for (var i = 0; i < textRuns.length; i++) {
    var run = textRuns[i];
    var text = run.asString();
    // Strip trailing newline that Slides appends
    if (text.endsWith('\n')) {
      text = text.substring(0, text.length - 1);
    }
    if (text.length === 0) {
      offset += run.asString().length;
      continue;
    }

    var font = run.getTextStyle().getFontFamily();
    var endOffset = offset + text.length - 1;

    runs.push({
      id: 'slides:S' + slideIdx + ':SH' + shapeIdx + ':C' + offset + '-' + endOffset,
      text: text,
      font: font
    });

    offset += run.asString().length;
  }

  return runs;
}
