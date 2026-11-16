> ## Documentation Index
> Fetch the complete documentation index at: https://docs.typesafe.ai/llms.txt
> Use this file to discover all available pages before exploring further.

# Models

Jev is TypeSafe's flagship model and the first [System One model](/concepts/system-one). Every model on this page is served by the same endpoint, `POST /v1/systemone`. The request's `model` field selects which one handles the call; see the [API reference](/api) for the full request shape.

## Current models

| Jev 1.13                    | `jev-1.13.0`                                                                              |
| :-------------------------- | :---------------------------------------------------------------------------------------- |
| Price (per Btok / per Mtok) | \$42 / \$0.042                                                                            |
| Rate limits                 | 250,000 tokens per second / 1,200 requests per minute                                     |
| Context length              | 64k tokens per request; 32k tokens for `state` plus the longest question                  |
| Input                       | Text only. String, JSON object, or array of text values. No image, audio, or video input. |

* **Price:** Charged per input token. Output tokens are free. A Btok is a billion tokens and an Mtok is a million tokens.
* **Rate limits:** Measured in tokens per second and requests per minute. A request over either limit returns `429 Too Many Requests`. Our [client SDKs](/sdk) retry with backoff by default and honor the `retry-after` header when the response carries one. If you call the HTTP API directly, see [Handling rate limits](/api#handling-rate-limits).
* **Context length:** Jev ingests the `state` once and evaluates every question against it in parallel. The 64k budget covers the `state` plus all questions combined; the 32k budget applies to the `state` plus the single longest question. See [Speculative fan-out](/patterns/fan-out) for packing many questions into one request, and [Jev 1.13 jaggedness](/model-jaggedness/jev-1.13) for how accuracy shifts as the state grows.
* **Input:** Jev evaluates natural-language text. Pre-process non-text inputs (images, audio, video, binaries) into text or structured fields before sending them as `state`. See [State](/concepts/state) for supported shapes.

<Warning>
  **Rate limits are adjusting dynamically.** We are serving a very large volume of demand, and the limits above can change without notice while we do, as upcoming large GPU deals land and we let in more users. Once things settle down more, we'll be able to offer more stable limits. Higher limits are available on custom and enterprise plans. Contact [sales@typesafe.ai](mailto:sales@typesafe.ai).
</Warning>

## Aliases

An alias is a model name that resolves to a versioned model ID. Send it in the `model` field like any other name.

| Alias         | Points to    | Meaning                                                                                                                       |
| :------------ | :----------- | :---------------------------------------------------------------------------------------------------------------------------- |
| `jev-latest`  | `jev-1.13.0` | The most recent stable, official release. The default in our client SDKs, and the name the examples in these docs use.        |
| `jev-preview` | `jev-1.13.0` | The most recent release, whether or not it is an official one. Moves ahead of `jev-latest` when a preview build is available. |

<Warning>
  `jev-preview` currently points to the same model as `jev-latest`. There is no preview build available right now.
</Warning>

An alias moves when a new release ships, so the answers behind it can change without a change on your side. The response's `model` field reports the versioned ID that answered, so you can log which model produced each result. If you have tuned confidence thresholds against a specific version, pin that version's ID instead of the alias and move to the new one on your own schedule.

## Customizing Jev

Jev is not fine-tuned or LoRA-adapted with customer data. It is trained with [RLCD](/introduction/machine-learning-primer) to return calibrated decisions, and the same weights serve every account. You shape its answers to your domain through the request rather than through per-account weights:

* Put your proprietary content, records, and reference material in the `state` field. See [State](/concepts/state).
* Encode your domain rules and boundary cases in the `instructions` and `criteria` of each question. See [How to build with TypeSafe](/concepts/how-to-build-with-system-one) and [Advanced: structure](/primitives/advanced).
* Decompose broad judgments into atomic questions and combine the outputs in code. See [Composite scoring](/patterns/composite-scoring) and the [AutoResearch cookbook](/cookbooks/autoresearch_feature_discovery) for training a downstream classical model on Jev's probabilities.

## Language support

Jev accepts natural-language text. English is the primary training language and where accuracy is currently best. Other languages, including CJK scripts, are handled but not equally well; test on your own content before relying on Jev for a non-English workload, and pay close attention to [Confidence](/confidence) when routing.

## Data handling

Jev is not trained on customer requests or responses. See [Legal](/legal) for the Data Processing Agreement, the Privacy Policy, and details on zero data retention (ZDR) for enterprise customers.

## Listing models

`GET /v1/models` returns the names your account can send in the `model` field, with a description and release date for each. It currently lists the aliases. Versioned IDs such as `jev-1.13.0` are accepted by the `model` field whether or not they appear in the list.

<CodeGroup>
  ```bash cURL theme={null}
  curl https://api.typesafe.ai/v1/models \
    -H "Authorization: Bearer $TYPESAFE_API_KEY"
  ```

  ```python Python theme={null}
  from typesafe_sdk import TypeSafeClient

  with TypeSafeClient() as client:
      for model in client.models.list().models:
          print(model.name, model.release_date, model.description)
  ```

  ```typescript JavaScript theme={null}
  import { TypeSafeClient } from "@typesafe-ai/sdk";

  const client = new TypeSafeClient();
  const models = await client.models.list();
  for (const model of models) {
    console.log(model.name, model.release_date, model.description);
  }
  ```
</CodeGroup>

<ResponseField name="models" type="array" required>
  One entry per model or alias.

  <Expandable title="properties">
    <ResponseField name="name" type="string" required>
      The model ID or alias, as accepted by the `model` field.
    </ResponseField>

    <ResponseField name="description" type="string" required>
      What the model is for.
    </ResponseField>

    <ResponseField name="release_date" type="string" required>
      When the model or alias was released.
    </ResponseField>
  </Expandable>
</ResponseField>

See the [Python](/sdk/python/api/clients/sync#typesafe_sdk.Models.list) and [JavaScript](/sdk/javascript/api/interfaces/Models) SDK references for the full method signatures.
