/**
 * Converts iconfont.cn font files to a type format for components/icon.
 *
 * Usage:
 * node path/to/ic.js -d path/to/iconfont-dir [-o output, default is ../components/icon]
 *
 * If no save location is specified, the file will be saved to ../components/icon
 * If a file with the same name exists, it will be overwritten.
 */
import { copyFileSync, existsSync, readFileSync, writeFileSync } from "fs";
import { join, dirname } from 'path';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const projectRoot = join(__dirname, '..', '..');

/**
 * Parse command line arguments and return configuration object
 * @returns {{iconfontDir: string, output: string}}
 */
function parseArguments() {
  let argv = process.argv.splice(2);
  let iconfontDir = '';
  let output = join(projectRoot, 'src', 'components', 'icon');

  argv.forEach((x, idx) => {
    if (x == '-d') {
      iconfontDir = argv[idx + 1];
    } else if (x == '-o') {
      output = argv[idx + 1];
    } else if (x == '-h') {
      showHelp();
      process.exit();
    }
  });

  if (!iconfontDir) {
    console.error('Please specify the iconfont file directory using the -d parameter');
    process.exit();
  }
  if (!existsSync(iconfontDir)) {
    console.error('The specified iconfont directory does not exist: ' + iconfontDir);
    process.exit();
  }
  return { iconfontDir, output };
}

/**
 * Process iconfont JSON file and generate type definitions
 * @param {string} iconfontDir
 * @param {string} output
 * @returns {string[]} List of logo class names
 */
function processIconFontJson(iconfontDir, output) {
  const jsonFile = join(iconfontDir, 'iconfont.json');
  const logoClass = [];

  if (!jsonFile) {
    console.error('The specified directory does not contain an iconfont.json file, or the file is not readable. Directory: ' + iconfontDir);
    process.exit();
  }

  let data = readFileSync(jsonFile);
  let jsonStr = data.toString().replace(/"description"\s*\:\s*"[^"]+"\s*,/i, '');
  let json = JSON.parse(jsonStr);

  if (json.glyphs) {
    let types = {};
    json.glyphs.forEach(x => {
      types[x.font_class] = '\\' + 'u' + x.unicode;
      if (x.font_class.startsWith('logo-') || x.font_class.startsWith('ai-')) {
        // console.log(x.font_class);
        logoClass.push(x.font_class);
      }
    });

    let jsonOutput = join(output, 'type.js');
    writeFileSync(jsonOutput, 'export default ' + JSON.stringify(types).replace(/\\+/g, '\\'));
    console.log('JSON file has been saved to: ' + jsonOutput);
  }

  return logoClass;
}

/**
 * Copy and process font files
 * @param {string} iconfontDir
 * @param {string} output
 */
function processFontFiles(iconfontDir, output) {
  // Copy woff2 file
  let woff2File = join(output, 'iconfont.woff2');
  copyFileSync(join(iconfontDir, 'iconfont.woff2'), woff2File);
  console.log('iconfont.woff2 file has been copied to: ' + woff2File);

  // Process CSS file
  let cssFile = join(iconfontDir, 'iconfont.css');
  let cssData = readFileSync(cssFile);
  let cssContent = cssData.toString().replace(
    /src\:\s+[\s\S]+?;/g,
    'src: url(\'@/components/icon/iconfont.woff2\') format(\'woff2\');'
  );

  let cssOutput = join(output, 'chatspeed.css');
  writeFileSync(cssOutput, cssContent);
  console.log('CSS file has been saved to: ' + cssOutput);
}

/**
 * Process logo related files and generate SVG content
 * @param {string[]} logoClass
 * @param {string} iconfontDir
 * @param {string} output
 */
function processLogoFiles(logoClass, iconfontDir, output) {
  if (logoClass.length === 0) return;

  // Write logo class file
  let logoOutput = join(output, 'logo.js');
  writeFileSync(logoOutput, 'export default ' + JSON.stringify(logoClass));
  console.log('Logo file has been saved to: ' + logoOutput);

  // Process SVG content
  let iconfontData = readFileSync(join(iconfontDir, 'iconfont.js'));
  let logoSvg = [];

  logoClass.forEach(x => {
    let svgContent = iconfontData.toString().match(
      new RegExp(`<symbol\\s+id="cs-${x}"\\s+viewBox="[^"]+"[>]*>[\\s\\S]+?<\\/symbol>`, 'g')
    );
    if (svgContent) {
      let newName = x.replace('ai-', 'logo-');
      svgContent = svgContent[0].replace(new RegExp(`id="cs-${x}"`, 'g'), `id="cs-${newName}"`);
      logoSvg.push(svgContent.trim());
    }
  });

  if (logoSvg.length > 0) {
    let logoSvgOutput = join(projectRoot, 'public', 'logoSvg.js');
    writeFileSync(
      logoSvgOutput,
      'var logoSvg=\'<svg>' + logoSvg.join('') + '</svg>\';' +
      // Injection script remains unchanged
      '(c=>{var l=(a=(a=document.getElementsByTagName("script"))[a.length-1]).getAttribute("data-injectcss"),a=a.getAttribute("data-disable-injectsvg");if(!a){var h,t,z,i,v,p=function(l,a){a.parentNode.insertBefore(l,a)};if(l&&!c.__iconfont__svg__cssinject__){c.__iconfont__svg__cssinject__=!0;try{document.write("<style>.svgfont {display: inline-block;width: 1em;height: 1em;fill: currentColor;vertical-align: -0.1em;font-size:16px;}</style>")}catch(l){console&&console.log(l)}}h=function(){var l,a=document.createElement("div");a.innerHTML=logoSvg,(a=a.getElementsByTagName("svg")[0])&&(a.setAttribute("aria-hidden","true"),a.style.position="absolute",a.style.width=0,a.style.height=0,a.style.overflow="hidden",a=a,(l=document.body).firstChild?p(a,l.firstChild):l.appendChild(a))},document.addEventListener?~["complete","loaded","interactive"].indexOf(document.readyState)?setTimeout(h,0):(t=function(){document.removeEventListener("DOMContentLoaded",t,!1),h()},document.addEventListener("DOMContentLoaded",t,!1)):document.attachEvent&&(z=h,i=c.document,v=!1,s(),i.onreadystatechange=function(){"complete"==i.readyState&&(i.onreadystatechange=null,m())})}function m(){v||(v=!0,z())}function s(){try{i.documentElement.doScroll("left")}catch(l){return void setTimeout(s,50)}m()}})(window);'
    );
    console.log('Logo SVG file has been saved to: ' + logoSvgOutput);
  }
}

/**
 * Display help information
 */
function showHelp() {
  console.log('\nConvert iconfont.cn JSON files to a usable type.js file for the project\nConverted files will be placed in /components/icon/type.js');
  console.log('Usage: ');
  console.log('\tnode ' + process.argv[1] + ' -d [iconfont dir] -o[output]');
  console.log('\t-d: Parameter to specify the directory of the files downloaded and extracted from iconfont.cn');
  console.log('\t-o: Output directory for the file. If not specified, the file will be stored in ../components/icon. If ../components/icon does not exist, it will be stored in the current directory ./\n');
}

// Main execution
try {
  const { iconfontDir, output } = parseArguments();
  const logoClass = processIconFontJson(iconfontDir, output);
  processFontFiles(iconfontDir, output);
  processLogoFiles(logoClass, iconfontDir, output);
} catch (error) {
  console.error('Error:', error);
  process.exit(1);
}
