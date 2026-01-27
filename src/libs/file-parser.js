import * as ExcelJS from 'exceljs';
import mammoth from 'mammoth';
import * as pdfjsLib from 'pdfjs-dist';

// Important: Import worker from local node_modules, Vite will handle this path automatically
// This ensures PDF parsing works even in offline or intranet environments
import pdfWorker from 'pdfjs-dist/build/pdf.worker.mjs?url';

pdfjsLib.GlobalWorkerOptions.workerSrc = pdfWorker;

/**
 * Parse Excel files (.xlsx, .xls)
 * Convert each Sheet to CSV string, which is the most AI-friendly structured text format
 */
export const parseExcel = async (file) => {
  const data = await file.arrayBuffer();
  const workbook = new ExcelJS.Workbook();
  await workbook.xlsx.load(data);
  let fullText = "";

  workbook.eachSheet((worksheet, sheetId) => {
    const sheetName = worksheet.name;
    let csv = '';

    worksheet.eachRow((row, rowNumber) => {
      const values = [];
      row.eachCell((cell, colNumber) => {
        // Get cell value and convert to string
        let cellValue = cell.value;
        if (cellValue === null || cellValue === undefined) {
          cellValue = '';
        } else if (typeof cellValue === 'object') {
          if (cellValue.result !== undefined) {
            // Handle formula result
            cellValue = cellValue.result;
          } else {
            // Handle other object types
            cellValue = String(cellValue);
          }
        }

        values.push(String(cellValue));
      });

      // Add row data to CSV string
      csv += values.join(',') + '\n';
    });

    if (csv.trim()) {
      fullText += `[Sheet: ${sheetName}]\n${csv}\n\n`;
    }
  });

  return fullText.trim();
};

/**
 * Parse Word files (.docx)
 * Mammoth focuses on extracting plain text, ignoring all complex styles and images, making it ideal for AI processing
 */
export const parseDocx = async (file) => {
  const arrayBuffer = await file.arrayBuffer();
  const result = await mammoth.extractRawText({ arrayBuffer });
  return result.value.trim();
};

/**
 * Parse PDF files
 * Iterate through all pages and extract text content
 */
export const parsePdf = async (file) => {
  const data = await file.arrayBuffer();
  const loadingTask = pdfjsLib.getDocument({ data });
  const pdf = await loadingTask.promise;
  let fullText = "";
  
  for (let i = 1; i <= pdf.numPages; i++) {
    const page = await pdf.getPage(i);
    const content = await page.getTextContent();
    // content.items contains text fragments on the page
    const strings = content.items.map(item => item.str);
    fullText += strings.join(" ") + "\n";
  }
  
  return fullText.trim();
};

/**
 * Universal file parsing dispatcher
 */
export const parseFileContent = async (file) => {
  const fileName = file.name.toLowerCase();
  
  if (fileName.endsWith('.pdf')) {
    return await parsePdf(file);
  } else if (fileName.endsWith('.xlsx') || fileName.endsWith('.xls')) {
    return await parseExcel(file);
  } else if (fileName.endsWith('.docx')) {
    return await parseDocx(file);
  } else {
    // For unsupported complex formats or plain text, try to read directly
    return await file.text();
  }
};
