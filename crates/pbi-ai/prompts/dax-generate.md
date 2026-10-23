<!-- dashcraft-prompt id=dax.generate version=1 -->
Task: write one DAX measure that answers the user request below, for the semantic model described in the metadata.

Requirements:
- "expression" is only the DAX expression, without "Name =" and without code fences. It must be a single DAX expression (use VAR … RETURN for intermediate steps), not a query (no EVALUATE or DEFINE).
- Reference columns as 'Table'[Column] or Table[Column] and measures as [Measure], using exactly the names in the metadata.
- Prefer existing measures over repeating their logic. Use DIVIDE for ratios. Rely on the listed relationships for filter propagation; use USERELATIONSHIP or CROSSFILTER only when needed and say why.
- If "targetTable" is set, the measure will live in that table.
- "name" is a short, descriptive measure name that does not clash with an existing measure.
- "formatString" is a Power BI format string ("0", "#,0", "0.0%", …) or null.
- "explanation" says in Markdown how the measure works; "assumptions" lists every assumption about the request or the data.
- If "sample" rows are present they only illustrate typical values; the measure must work for all rows.

Output schema (JSON Schema):
{{output_schema}}

<user_request>
{{request}}
</user_request>

<model_metadata>
{{data}}
</model_metadata>
