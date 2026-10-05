# R3-4 preregistration amendment 1 — unknown category coverage

**Status:** registered before any R3-4 result table was successfully generated
or inspected. This clarifies the already-frozen §4.1 rule that unknown category
values may appear only as a disclosure-safe aggregate coverage count.

The implementation will treat a linked complaint as not category-classifiable
when either `category` or `subcategory` is blank. It will not impute a category,
will exclude that case from every category-stratified table, and will report at
most one global count of linked cases missing either category field. It will not
separate the two missingness dimensions, publish a cohort denominator, or attach
other outcomes to an unknown-category bucket. The single count is suppressed
when below `k=10`; if safe, the output is marked aggregate coverage only.

This operationalizes the original preregistration's unknown-coverage clause; it
does not change the population, joins, allowlisted input columns, query-family
rules, outcome definitions, candidate questions, or interpretation. The result
envelope advances to schema version 2 solely to add the explicit
`bank_category_coverage` table.

