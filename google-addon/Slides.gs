/**
 * Entry point for Slides conversion.
 */
function showConvertDialogSlides() {
  var presentation = SlidesApp.getActivePresentation();
  var selection = presentation.getSelection();
  var runs;
  var scope;

  var selType = selection.getSelectionType();

  if (selType === SlidesApp.SelectionType.TEXT) {
    // User is editing text inside a shape — convert only that text range
    var textRange = selection.getTextRange();
    var pageElement = selection.getPageElementRange().getPageElements()[0];
    var slideShape = findSlideAndShapeIndex_(presentation, pageElement);
    runs = extractRunsFromTextRange_(textRange, slideShape.slide, slideShape.shape);
    scope = 'selection';
  } else if (selType === SlidesApp.SelectionType.PAGE_ELEMENT) {
    // User selected one or more shapes — convert text in those shapes
    var pageElements = selection.getPageElementRange().getPageElements();
    runs = extractRunsFromSelectedShapes_(presentation, pageElements);
    scope = 'selection';
  } else {
    runs = extractRunsFromAllSlides_(presentation);
    scope = 'document';
  }

  showConvertDialog_(runs, scope, 'slides');
}

/**
 * Extracts runs from a set of selected page elements (shapes).
 */
function extractRunsFromSelectedShapes_(presentation, pageElements) {
  var runs = [];
  for (var i = 0; i < pageElements.length; i++) {
    var el = pageElements[i];
    var textRange;
    try {
      textRange = el.asShape().getText();
    } catch (e) {
      continue;  // Not a shape or has no text frame
    }
    var idx = findSlideAndShapeIndex_(presentation, el);
    var shapeRuns = extractRunsFromTextRange_(textRange, idx.slide, idx.shape);
    runs = runs.concat(shapeRuns);
  }
  return runs;
}

/**
 * Finds the slide and shape index for a given page element.
 * Returns {slide: number, shape: number}.
 */
function findSlideAndShapeIndex_(presentation, pageElement) {
  var objectId = pageElement.getObjectId();
  var slides = presentation.getSlides();
  for (var s = 0; s < slides.length; s++) {
    var shapes = slides[s].getShapes();
    for (var sh = 0; sh < shapes.length; sh++) {
      if (shapes[sh].getObjectId() === objectId) {
        return { slide: s, shape: sh };
      }
    }
  }
  return { slide: 0, shape: 0 };  // fallback
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
