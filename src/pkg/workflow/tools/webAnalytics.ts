import { workflowCallTool } from '../api'
import { callLLM } from '../llm'
import type { ToolDefinition, WorkflowMessage } from '../types'

// Store the current workflow context
let currentWorkflowContext: {
  providerId: number
  modelId: string
} | null = null

/**
 * Set the current workflow context for WebAnalytics
 * This should be called by the workflow engine before executing tools
 * @param context The workflow context containing providerId and modelId
 */
export function setWebAnalyticsContext(context: { providerId: number; modelId: string }): void {
  currentWorkflowContext = context
}

export const webAnalyticsTool: ToolDefinition = {
  id: 'WebAnalytics',
  name: 'WebAnalytics',
  description: `Extracts and analyzes relevant information from web content based on a specific topic. This tool first fetches the webpage content and then uses AI to extract only the most relevant information, helping to avoid context overflow in workflows.

## Usage Guidelines
- Use this tool when you need to analyze web content for specific information
- The tool will fetch the webpage and extract only content relevant to your topic
- This helps prevent context overflow in long conversations
- Supports different analysis types: summary, key_points, or data_extraction

## Parameters
- topic: The specific topic or question you're interested in
- url: The webpage URL to analyze
- analysis_type: Type of analysis (optional, defaults to 'summary')`,

  inputSchema: {
    type: 'object',
    properties: {
      topic: {
        type: 'string',
        description: 'The topic or question to analyze the webpage content for'
      },
      url: {
        type: 'string',
        description: 'The URL of the webpage to analyze'
      },
      analysis_type: {
        type: 'string',
        enum: ['summary', 'key_points', 'data_extraction'],
        description: 'Type of analysis to perform. Defaults to "summary"',
        default: 'summary'
      }
    },
    required: ['topic', 'url']
  },
  implementation: 'typescript',
  requiresApproval: false, // This tool doesn't require user approval
  handler: async params => {
    // Validate parameters
    if (!params.topic || typeof params.topic !== 'string') {
      throw new Error('Topic is required and must be a string')
    }

    if (!params.url || typeof params.url !== 'string') {
      throw new Error('URL is required and must be a string')
    }

    const analysisType = (params.analysis_type as string) || 'summary'

    // Check if we have workflow context
    if (!currentWorkflowContext) {
      throw new Error('WebAnalytics tool is not initialized with workflow context')
    }

    try {
      // 1. Fetch webpage content using WebFetch tool
      const webFetchResult = await workflowCallTool('WebFetch', {
        url: params.url,
        format: 'markdown',
        keep_link: false,
        keep_image: false
      })

      if (!webFetchResult.success || !webFetchResult.result) {
        throw new Error(`Failed to fetch webpage: ${webFetchResult.error || 'Unknown error'}`)
      }

      // Extract content from the result
      let webContent: string
      if (typeof webFetchResult.result === 'string') {
        webContent = webFetchResult.result
      } else if (
        webFetchResult.result &&
        typeof webFetchResult.result === 'object' &&
        'content' in webFetchResult.result
      ) {
        webContent = (webFetchResult.result as any).content
      } else {
        throw new Error('No content returned from WebFetch')
      }

      // Check if content is too short
      if (webContent.length < 50) {
        throw new Error('Webpage content is too short or empty')
      }

      // 2. Build AI analysis prompt
      const analysisPrompt = buildAnalysisPrompt(params.topic, webContent, analysisType)

      // 3. Prepare messages for AI
      const messages: WorkflowMessage[] = [
        {
          sessionId: 'web-analytics',
          role: 'system',
          message:
            'You are a helpful assistant that analyzes web content and extracts relevant information based on user queries. Always provide concise, factual responses.'
        },
        {
          sessionId: 'web-analytics',
          role: 'user',
          message: analysisPrompt
        }
      ]

      // 4. Call AI for analysis
      let analysisResult = ''
      await callLLM(
        {
          providerId: currentWorkflowContext.providerId,
          modelId: currentWorkflowContext.modelId,
          messages,
          temperature: 0.3, // Lower temperature for more consistent analysis
          availableTools: [], // No tools for this AI call
          tsTools: []
        },
        {
          onContent: (chunk: string) => {
            analysisResult += chunk
          },
          onDone: () => {
            // Analysis complete
          },
          onError: (error: any) => {
            throw new Error(`AI analysis failed: ${error.message || String(error)}`)
          }
        }
      )

      // 5. Format and return result
      const formattedResult = {
        topic: params.topic,
        url: params.url,
        analysis_type: analysisType,
        analysis: analysisResult.trim(),
        content_length: webContent.length
      }

      return JSON.stringify(formattedResult, null, 2)
    } catch (error: any) {
      throw new Error(`WebAnalytics failed: ${error.message || String(error)}`)
    }
  }
}

/**
 * Build the analysis prompt based on the analysis type
 * @param topic The topic to analyze for
 * @param webContent The webpage content
 * @param analysisType The type of analysis to perform
 * @returns The analysis prompt
 */
function buildAnalysisPrompt(topic: string, webContent: string, analysisType: string): string {
  switch (analysisType) {
    case 'key_points':
      return `Please extract the key points from the following webpage content that are relevant to the topic: "${topic}".

Provide the key points as a concise bullet list.

Webpage Content:
${webContent}`

    case 'data_extraction':
      return `Extract specific data and facts from the following webpage content that relate to: "${topic}".

Focus on concrete information, statistics, and factual data.

Webpage Content:
${webContent}`

    case 'summary':
    default:
      return `Please provide a concise summary of the following webpage content, focusing on information relevant to: "${topic}".

Keep the summary brief and to the point.

Webpage Content:
${webContent}`
  }
}
