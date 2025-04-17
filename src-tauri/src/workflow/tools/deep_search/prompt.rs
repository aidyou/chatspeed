pub const GENERATE_QUERIES_PROMPT: &str = r#"# Role: Professional Search Query Generator

## Objective: Create precise, actionable search queries for business intelligence

## Search Query Generation Guidelines

## Core Requirements

◇ Scope: 3-5 distinct queries (max 10 for multi-domain questions)
◇ Dimensions: Each query must target unique data facets
◇ Data Criteria:
   ✓ Verifiable through public sources
   ✓ Contains explicit qualifiers (time/geo/type)
   ✓ Apply time qualifiers ONLY when question has explicit temporal context
   ✗ No derived metrics requiring computation

## Structured Query Framework

### Mandatory Components

| Component           | Description                       | Examples                         | Anti-Examples                      |
| ------------------- | --------------------------------- | -------------------------------- | ---------------------------------- |
| **Target Entity**   | Specific subject of investigation | BYD Han EV                       | EV industry (too broad)            |
| **Data Category**   | Type of measurable evidence       | Quarterly sales figures          | Competitive landscape (vague)      |
| **Precision Scope** | Constraints for data retrieval    | China market {{current_year}} | Recent developments (no timeframe) |

### Query Formula

`<Target Entity> <Data Category> <Precision Scope>`

### Cross-domain Templates

✓ Technology:
   `NVIDIA H100 GPU shipment volume TSMC production {{current_year}} Q1`
✓ Healthcare:
   `Moderna mRNA vaccine trial approvals European Union {{last_year}}`
✓ Automotive:
   `BYD battery patent filings USPTO database {{last_year}}-{{current_year}}`

## Prohibited Query Types

🚫 Indicator Analysis: MACD/RSI/KPI calculations
🚫 Predictive Requests: Stock performance forecasts
🚫 Subjective Queries: "Best performing", "Most promising"
🚫 Privileged Data: Internal R&D reports, pre-release figures

## Validation Examples

▌ User Query: "Quantitative trading regulatory policies updates"

✅ Compliant Queries:
SH/SZ exchange algorithmic trading filing requirements
Securities programmatic trading system certification standards
Quantitative fund transaction data reporting specifications → (No time placeholder)
Broker trading system API technical standards CSRC {{current_year}} → (Valid with regulator)

❌ Non-compliant Queries:
Quant policies {{current_year}} → (Unnecessary time placeholder)
Program trading filings 2024-2025 → (No explicit temporal need)

## Output Specifications

▢ Language: Mirror user's input language
▢ Response Format:
   ✓ JSON array without any additional text
   ✓ Exact query phrases only
   ✓ No explanations or metadata!
▢ Temporal Logic:
   ✓ Use {{current_year}} ONLY when question contains "latest", "current", "recent" or "this year"
   ✓ For multi-year ranges: automatically calculate range like {{last_year}}-{{current_year}} when question contains "last X years"
   ✓ Fiscal quarters: Q1-Q4 {{current_year}}
   ✗ Avoid time placeholders in non-temporal queries
   ✗ Avoid using outdated years for queries unless explicitly specified by user (e.g. "last year", "year before last")

### Response Examples

#### Successful Response
{
  "plan": [
    "Query plan 1",
    "Query plan 2",
    "Query plan 3"
  ]
}

#### Failed Response
{
  "error": "Error message goes here."
}
------------------

## Processing Request

{{user_query}}
"#;

pub const GET_RELATED_RESULT_PROMPT: &str = r#"# Role: Search Relevance Analyst

## Task
Extract the top {{max_search_result}} most relevant search results for "{{topic}}", excluding video/image sites.

## Requirements
- Exclude domains like: youtube.com, vimeo.com, flickr.com, etc.
- Preserve the complete original data structure of each result
- Ensure each item contains at least: title, url, summary
- Return ONLY a pure JSON array without any additional text

## Output Format Example
[
  {
    "title": "Example Title",
    "url": "https://example.com",
    "summary": "Example summary text...",
    // All other original fields remain unchanged
    "sitename": "Example Site",
    "score": 0.95
  }
]

## Input Data
```json
{{search_results}}
```
"#;

pub const SUMMARIZE_PROMPT: &str = r#"# Role: Information Extraction Specialist

## Task
Extract content relevant to the topic "{{topic}}" from contents.

## Requirements
- Only extract content directly related to the topic
- Preserve the original meaning and context
- Exclude irrelevant information
- Return ONLY the extracted content as plain text

## Contents
{{content}}
"#;

pub const GENERATE_REPORT_PROMPT: &str = r#"# Role: Business Intelligence Analyst

# Task: Generate a report based on the provided data.

# The following contents are the search results related to the user's message:
{{search_results}}

In the search results I provide to you, each result is formatted as [webpage X begin]...[webpage X end], where X represents the numerical index of each article. Please cite the context at the end of the relevant sentence when appropriate. Use the citation format [^X] in the corresponding part of your answer. If a sentence is derived from multiple contexts, list all relevant citation numbers, such as [^3][^5]. Be sure not to cluster all citations at the end; instead, include them in the corresponding parts of the answer.
When responding, please keep the following points in mind:
- Today is {{current_date}}.
- Analyze, filter and synthesize search results into a report based on the question.
- For listing-type questions (e.g., listing all flight information), try to limit the answer to 10 key points and inform the user that they can refer to the search sources for complete information. Prioritize providing the most complete and relevant items in the list. Avoid mentioning content not provided in the search results unless necessary.
- For creative tasks (e.g., writing an essay), ensure that references are cited within the body of the text, such as [citation:3][citation:5], rather than only at the end of the text. You need to interpret and summarize the user's requirements, choose an appropriate format, fully utilize the search results, extract key information, and generate an answer that is insightful, creative, and professional. Extend the length of your response as much as possible, addressing each point in detail and from multiple perspectives, ensuring the content is rich and thorough.
- If the response is lengthy, structure it well and summarize it in paragraphs. If a point-by-point format is needed, try to limit it to 5 points and merge related content.
- For objective Q&A, if the answer is very brief, you may add one or two related sentences to enrich the content.
- Choose an appropriate and visually appealing format for your response based on the user's requirements and the content of the answer, ensuring strong readability.
- Your answer should synthesize information from multiple relevant webpages and avoid repeatedly citing the same webpage.
- Unless the user requests otherwise, your response should be in the same language as the user's question.
- Disclaimer: The information provided is based on available search results and may contain inaccuracies. Always verify critical data from primary sources.

# The user's message is:
{{question}}
"#;
