import * as XLSX from 'xlsx';
import mammoth from 'mammoth';
import * as pdfjsLib from 'pdfjs-dist';

// 重要：从本地 node_modules 引入 worker，Vite 会自动处理这个路径
// 这保证了在断网或内网环境下 PDF 解析依然可用
import pdfWorker from 'pdfjs-dist/build/pdf.worker.mjs?url';

pdfjsLib.GlobalWorkerOptions.workerSrc = pdfWorker;

/**
 * 解析 Excel 文件 (.xlsx, .xls)
 * 将每个 Sheet 转换为 CSV 字符串，这是 AI 最容易理解的结构化文本格式
 */
export const parseExcel = async (file) => {
  const data = await file.arrayBuffer();
  const workbook = XLSX.read(data);
  let fullText = "";
  
  workbook.SheetNames.forEach(sheetName => {
    const worksheet = workbook.Sheets[sheetName];
    // 使用 CSV 格式，因为它比 JSON 更省Token 且保留了结构感
    const csv = XLSX.utils.sheet_to_csv(worksheet);
    if (csv.trim()) {
      fullText += `[Sheet: ${sheetName}]\n${csv}\n\n`;
    }
  });
  
  return fullText.trim();
};

/**
 * 解析 Word 文件 (.docx)
 * Mammoth 专注于提取纯文本，忽略所有复杂的样式和图片，非常适合 AI 处理
 */
export const parseDocx = async (file) => {
  const arrayBuffer = await file.arrayBuffer();
  const result = await mammoth.extractRawText({ arrayBuffer });
  return result.value.trim();
};

/**
 * 解析 PDF 文件
 * 遍历所有页面并提取文本内容
 */
export const parsePdf = async (file) => {
  const data = await file.arrayBuffer();
  const loadingTask = pdfjsLib.getDocument({ data });
  const pdf = await loadingTask.promise;
  let fullText = "";
  
  for (let i = 1; i <= pdf.numPages; i++) {
    const page = await pdf.getPage(i);
    const content = await page.getTextContent();
    // content.items 包含了页面上的文本片段
    const strings = content.items.map(item => item.str);
    fullText += strings.join(" ") + "\n";
  }
  
  return fullText.trim();
};

/**
 * 通用文件解析分发器
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
    // 对于不支持的复杂格式或普通文本，尝试直接读取
    return await file.text();
  }
};
