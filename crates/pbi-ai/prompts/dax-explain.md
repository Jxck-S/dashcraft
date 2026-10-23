<!-- dashcraft-prompt id=dax.explain version=1 -->
Task: explain the DAX measure described by "measure" in the metadata below to a Power BI report author.

Cover:
- what the measure returns, in one or two sentences ("summary");
- how the calculation works step by step, in Markdown ("explanation"): functions used, iterators, variables, and how the referenced measures ("referencedMeasures") contribute;
- filter context: which filters the measure keeps, removes or adds (CALCULATE, ALL, REMOVEFILTERS, KEEPFILTERS, …), any context transition (a measure or CALCULATE inside a row context), and how filters reach the referenced tables through the relationships listed ("filterContext");
- one short note per dependency: measure as `[Name]`, column as `Table[Column]` ("dependencies");
- pitfalls: BLANK results, division by zero, inactive or bidirectional relationships, many-to-many paths, data type conversions, performance ("caveats").

If "notes" reports that the DAX does not parse, explain what it appears to intend and say that it does not parse.

Output schema (JSON Schema):
{{output_schema}}

<model_metadata>
{{data}}
</model_metadata>
