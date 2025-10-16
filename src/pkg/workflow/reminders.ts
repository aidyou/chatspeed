/**
 * This file contains all the system-reminder messages used in the WorkflowEngine.
 */

// --- Success Reminders ---

export const WEB_SEARCH_NO_RESULTS = 
  '<system-reminder>No results found. Consider changing your keywords, adjusting the time range.</system-reminder>'

export const WEB_SEARCH_SUCCESS = 
  '<system-reminder>Analyze the results to find the most relevant link, then use the WebFetch tool to get detailed content.</system-reminder>'

export const WEB_FETCH_INSUFFICIENT_CONTENT = 
  '<system-reminder>Failed to fetch significant content. The page may be protected, require JavaScript, or be empty. If other URLs are available, try a different one.</system-reminder>'

export const WEB_FETCH_SUCCESS = 
  "<system-reminder>Content successfully fetched. Extract the key information relevant to the user's query or goal.</system-reminder>"

export const TODO_MANAGER_SUCCESS = 
  '<system-reminder>Great! The todo list is updated. Keep maintaining a clear plan.</system-reminder>'

export const TODO_USAGE_REMINDER = 
  "<system-reminder>You haven't used the todo tool in several steps. Please review your task plan and manage the todo list if necessary.</system-reminder>"

// --- Error Reminders ---

export const INVALID_PARAMS_ERROR = 
  "<system-reminder>Tool call failed due to invalid parameters. Check the tool's documentation for the correct parameter schema and try again.</system-reminder>"

export const FATAL_ERROR = 
  '<system-reminder>A fatal error occurred. The workflow cannot continue and will now be terminated.</system-reminder>'

// --- Dynamic Error Reminders ---

export const networkErrorRetry = (attempt: number, max: number) => 
  `<system-reminder>A network error occurred. Retrying the operation. (Attempt ${attempt} of ${max})</system-reminder>`

export const networkErrorMaxRetries = (max: number) => 
  `<system-reminder>A network error occurred and the maximum number of retries (${max}) has been reached. The workflow will be terminated.</system-reminder>`

export const unexpectedError = (message: string) => 
  `<system-reminder>An unexpected error occurred while executing the tool: ${message}</system-reminder>`
