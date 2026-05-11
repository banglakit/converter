/**
 * Adds the Banglakit Converter menu when a Sheet is opened.
 */
function onOpen() {
  SpreadsheetApp.getUi()
    .createAddonMenu()
    .addItem('Convert to Unicode', 'showConvertDialogSheets')
    .addToUi();
}

/**
 * Entry point for Sheets conversion.
 */
function showConvertDialogSheets() {
  var sheet = SpreadsheetApp.getActiveSpreadsheet();
  var selection = sheet.getSelection();
  var activeRange = selection.getActiveRange();
  var runs;
  var scope;

  // If user selected specific cells, use those; otherwise use all data
  if (activeRange && !isEntireSheet_(activeRange, sheet.getActiveSheet())) {
    runs = extractRunsFromRange_(activeRange);
    scope = 'selection';
  } else {
    var dataRange = sheet.getActiveSheet().getDataRange();
    runs = extractRunsFromRange_(dataRange);
    scope = 'document';
  }

  showConvertDialog_(runs, scope, 'sheets');
}

/**
 * Checks if the active range covers the entire sheet (i.e., no meaningful selection).
 */
function isEntireSheet_(range, sheet) {
  return range.getNumRows() >= sheet.getMaxRows() &&
         range.getNumColumns() >= sheet.getMaxColumns();
}

/**
 * Extracts {id, text, font} from each non-empty cell in a range.
 * ID format: "sheets:R<row>:C<col>" (1-based)
 */
function extractRunsFromRange_(range) {
  var runs = [];
  var values = range.getValues();
  var fonts = range.getFontFamilies();
  var startRow = range.getRow();
  var startCol = range.getColumn();

  for (var r = 0; r < values.length; r++) {
    for (var c = 0; c < values[r].length; c++) {
      var val = values[r][c];
      if (val === '' || val === null || val === undefined) continue;
      // Only process string values
      if (typeof val !== 'string') continue;

      runs.push({
        id: 'sheets:R' + (startRow + r) + ':C' + (startCol + c),
        text: val,
        font: fonts[r][c]
      });
    }
  }

  return runs;
}
