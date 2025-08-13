/**
 * Utility functions for common operations in the application.
 *
 * This file includes various helper functions that assist with:
 * 1. Checking if an object is empty (`isEmpty`).
 * 2. Displaying messages using Element Plus (`showMessage`).
 * 4. Managing local storage with type information (`csSetStorage`, `csGetStorage`, `csRemoveStorage`).
 * 5. Converting values to integers and floats (`toInt`, `toFloat`).
 * 6. Converting camelCase strings to snake_case (`camelToSnake`).
 * 7. Converting snake_case strings to camelCase (`snakeToCamel`).
 *
 * These utility functions are designed to promote code reusability and maintainability throughout the application.
 * Future adjustments and additions may be made as needed to accommodate new requirements or functionalities.
 */

import { openUrl as invokeOpenUrl } from '@tauri-apps/plugin-opener'
import { ElMessage } from 'element-plus';
import 'element-plus/es/components/message/style/css';

/**
 * Checks if the given object is empty.
 * An object is considered empty if:
 * - It is null or undefined.
 * - It is an empty array.
 * - It is an empty Map.
 * - It is an empty Set.
 * - It is a plain object with no enumerable properties.
 *
 * @param {any} o - The object to check.
 * @returns {boolean} - Returns true if the object is empty, otherwise false.
 */
export function isEmpty(o) {
  // Check if the object is null or undefined
  if (o === null || typeof o === 'undefined') {
    return true;
  }

  // Check if the object is an array and if it is empty
  if (Array.isArray(o)) {
    return o.length === 0; // Only need to check if it's an empty array
  }

  // Check if the object is a Map and if it is empty
  if (o instanceof Map) {
    return o.size === 0; // Empty Map
  }

  // Check if the object is a Set and if it is empty
  if (o instanceof Set) {
    return o.size === 0; // Empty Set
  }

  // Check if the object is a plain object
  if (typeof o === 'object') {
    // Check if it is a plain object and has no enumerable properties
    if (Object.prototype.toString.call(o) === '[object Object]') {
      for (const key in o) {
        // Check if the property is a direct property of the object
        if (Object.prototype.hasOwnProperty.call(o, key)) {
          return false; // Found at least one property, return false
        }
      }
      return true; // No properties found, return true
    }
  }
  // Handle other types (numbers, booleans, functions, etc.)
  return !o; // Return true if falsy (0, '', false), otherwise false
}


/**
 * show el-message
 * @param {string} message - The message to display.
 * @param {string} type - The type of message to display.
 * @param {string} duration - The duration of the message.
 */
export function showMessage(message, type = 'info', duration = 0) {
  ElMessage({
    message: message,
    type: type,
    duration: duration < 1 ? (type == 'error' || type == 'warn' ? 5000 : 3000) : duration,
    offset: 40
  })
}
/**
 * show message box
 * @param {string} message - The message to display.
 * @param {string} type - The type of message to display.
 * @param {string} duration - The duration of the message.
 */
export function showMessageBox(message, type = 'info', duration = 5000) {
  let title = type.charAt(0).toUpperCase() + type.slice(1);
  if (type === 'primary') {
    title = 'Info'
  }
  ElNotification({
    title,
    message,
    type,
    duration,
    offset: 40
  })
}

/**
 * Writes data to the browser's local storage with type information.
 * @param {string} key - The key for storage.
 * @param {any} value - The value to store locally.
 * @param {string} keyPrefix - The prefix for the storage key.
 */
export function csSetStorage(key, value, keyPrefix = '__cs_') {
  let t = typeof value;

  // Handle special types
  if (value === null) {
    t = 'null';
  } else if (Array.isArray(value)) {
    t = 'array';
  } else if (value instanceof Date) {
    t = 'date';
  } else if (typeof value === 'object') {
    t = 'object';
  }

  localStorage.setItem((keyPrefix || '') + key, JSON.stringify({ t: t, d: value }));
}

/**
 * Retrieves data from the browser's local storage and parses it based on type information.
 * @param {string} key - The key for local storage.
 * @returns {any} - The value stored in local storage.
 * @param {string} defaultValue - The default value to return if the key does not exist.
 * @param {string} keyPrefix - The prefix for the storage key.
 */
export function csGetStorage(key, defaultValue = null, keyPrefix = '__cs_') {
  const storedData = localStorage.getItem((keyPrefix || '') + key);

  if (storedData === null) {
    return defaultValue; // Data does not exist
  }

  try {
    const parsedData = JSON.parse(storedData);
    const { t, d } = parsedData;

    // Convert data back to original format based on type information
    switch (t) {
      case 'boolean':
        return Boolean(d);
      case 'number':
        return Number(d);
      case 'string':
        return String(d);
      case 'object':
        return d; // Already an object, no conversion needed
      case 'array':
        return d; // Already an array, no conversion needed
      case 'date':
        return new Date(d); // Convert to date object
      case 'null':
        return null;
      default:
        return d; // If type is unknown, return original data
    }
  } catch (e) {
    // If parsing fails, return the original string
    return storedData;
  }
}

/**
 * Deletes data from the browser's local storage.
 * @param {string} key - The key for storage.
 */
export function csRemoveStorage(key, keyPrefix = '__cs_') {
  localStorage.removeItem((keyPrefix || '') + key);
}


/**
 * Convert value to integer
 * @param {any} value - The value to convert.
 * @returns {number} - The converted integer value.
 */
export function toInt(value) {
  if (typeof value === 'number') {
    return value
  }
  return parseInt(value, 10)
}

/**
 * Convert value to float
 * @param {any} value - The value to convert.
 * @returns {number} - The converted float value.
 */
export function toFloat(value) {
  if (typeof value === 'number') {
    return value
  }
  return parseFloat(value)
}

/**
 * Converts a camelCase string to snake_case
 *
 * # Examples:
 * - camelToSnake("helloWorld") returns "hello_world"
 * - camelToSnake("HelloWorld") returns "hello_world"
 * - camelToSnake("HELLO_WORLD") returns "hello_world"
 * - camelToSnake("JSONData") returns "json_data"
 *
 * @param {string} str - The camelCase string to convert
 * @returns {string} The converted snake_case string
 */
export function camelToSnake(str) {
  return str
    // Preserve existing underscores
    .split('_')
    .map(part =>
      part
        // Handle consecutive uppercase letters (e.g., "JSON" -> "json")
        .replace(/([A-Z]+)([A-Z][a-z])/g, '$1_$2')
        // Insert underscore before capital letters
        .replace(/([a-z\d])([A-Z])/g, '$1_$2')
        .toLowerCase()
    )
    .join('_')
    // Clean up any duplicate underscores
    .replace(/_+/g, '_')
    // Remove leading/trailing underscores
    .replace(/^_|_$/g, '');
}

/**
 * Converts a snake_case string to camelCase
 *
 * # Examples:
 * - snakeToCamel("hello_world") returns "helloWorld"
 * - snakeToCamel("HELLO_WORLD") returns "helloWorld"
 * - snakeToCamel("primary_language") returns "primaryLanguage"
 * - snakeToCamel("primary_LANGUAGE") returns "primaryLanguage"
 * - snakeToCamel("hello__world") returns "helloWorld"
 *
 * @param {string} str - The snake_case string to convert
 * @returns {string} The converted camelCase string
 */
export function snakeToCamel(str) {
  // Split the string by underscores and handle each part
  return str
    .split('_')
    .filter(Boolean) // Remove empty parts from multiple underscores
    .map((part, index) => {
      // Capitalize first letter if it's not the first word
      if (index > 0) {
        part = part.charAt(0).toUpperCase() + part.slice(1);
      }
      return part;
    })
    .join('');
}


/**
 * Converts a timestamp to a formatted date string.
 * The date format is `YYYY-MM-DD HH:mm:ss`.
 *
 * @param {integer} timestamp - The timestamp to convert.
 * @returns {string} The formatted date string.
 */
export function formatTime(timestamp) {
  const date = new Date(timestamp)
  const year = date.getFullYear()
  const month = String(date.getMonth() + 1).padStart(2, '0')
  const day = String(date.getDate()).padStart(2, '0')
  const hour = String(date.getHours()).padStart(2, '0')
  const minute = String(date.getMinutes()).padStart(2, '0')
  const second = String(date.getSeconds()).padStart(2, '0')
  return `${year}-${month}-${day} ${hour}:${minute}:${second}`
}


const hexTable = Array.from({ length: 256 }, (_, i) =>
  i.toString(16).padStart(2, '0')
);
/**
 * Generates a UUID (Universally Unique Identifier).
 * The UUID is generated using a random number generator.
 *
 * @returns {string} The generated UUID.
 */
export function Uuid() {
  const buf = crypto.getRandomValues(new Uint8Array(16));
  buf[6] = (buf[6] & 0x0f) | 0x40;
  buf[8] = (buf[8] & 0x3f) | 0x80;

  return hexTable[buf[0]] + hexTable[buf[1]] +
    hexTable[buf[2]] + hexTable[buf[3]] + '-' +
    hexTable[buf[4]] + hexTable[buf[5]] + '-' +
    hexTable[buf[6]] + hexTable[buf[7]] + '-' +
    hexTable[buf[8]] + hexTable[buf[9]] + '-' +
    hexTable[buf[10]] + hexTable[buf[11]] +
    hexTable[buf[12]] + hexTable[buf[13]] +
    hexTable[buf[14]] + hexTable[buf[15]];
}

/**
 * Opens the given URL in the default web browser
 */
export async function openUrl(url) {
  try {
    await invokeOpenUrl(url)
  } catch (error) {
    console.log(error)
    showMessage(t('common.openUrlError'), 'error')
  }
}