## What changed

Moved the **Jira** client under `provider_boards` and added _write_ support.

- Create, update and ~~delete~~ move issues
- Comments use [ADF](https://developer.atlassian.com/cloud/jira/platform/apis/document/structure/ "ADF docs")
  1. plain text
  2. code: `x < y && z`

```rust
pub fn markdown_to_adf(markdown: &str) -> Value
```

> Note: rate limits apply (100 req/s).
> See <https://developer.atlassian.com/cloud/jira/platform/rate-limiting/>.

---

Closes #101 and fixes a *snake_case* bug in `issue_key`.
