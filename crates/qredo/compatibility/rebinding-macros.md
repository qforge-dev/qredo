# EX4028 call heads are not variable bindings

Pinned Credo `ea1ccb9` collects only direct assignment statements in a checked
block. `assert {:ok, value} = call()` is an `assert` macro call containing a
match argument; neither `assert` nor that argument is a direct binding for
this check.

The previous text scanner reported both `assert` and `value` after two such
statements. `EX4028.call-head` reproduced the false positive before the fix.
The scanner now uses the existing shared call-start inventory to distinguish
call statements from assignments. This applies to local, remote and custom
macro calls; assertions remain intact. `assert = 1; assert = 2` still reports
the real variable (dedicated regression test).

The repeated-assert example was verified clean against the pinned native check.
All existing rebinding corpus cases continue to pass.

`EX4028.unicode-offsets` also covers non-ASCII test names/comments before macro
calls. Since masking preserves character positions rather than byte offsets,
the call inventory is translated into masked-buffer offsets in one pass;
ASCII source keeps the direct-offset fast path.
