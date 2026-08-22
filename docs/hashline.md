# Hashline patch grammar

When `edit_mode` is `"hashline"`, `edit` accepts exactly one argument:

```json
{ "patch": "..." }
```

A patch contains one or more tagged file sections. An optional
`*** Begin Patch` / `*** End Patch` envelope may wrap the sections.

```text
[path/to/file#TAG]
OPERATION
```

`TAG` is exactly four hexadecimal digits. Obtain it from a current tagged `read`
of that path. A tagged read retains only the rows it shows, so each line address
must be within the read result. Gap addresses also require the adjacent rows,
and `REM` and `MV` require a whole-file tagged read. Tags rotate after a mutation;
read the file again before a follow-up patch. An edit response can show a tag for
changed context, but it may not retain all rows needed by the next operation.

## Addresses

All addresses use the coordinates from the tagged read, even when a section has
multiple operations.

| Form | Meaning |
| --- | --- |
| `0` | Beginning-of-file insertion point. |
| `N` | One line. With `PUT`, this replaces line `N`. |
| `N.=M` | Inclusive range. `N..=M` and `N..M` are also accepted. |
| `<N` / `>N` | Insertion gap before / after line `N`. Use these to insert without replacing a line. |
| `N*` | Language-aware block containing line `N`. |
| `<N*` / `>N*` | Insertion gap before / after that block. |
| `$` / `$-K` | Last line / `K` lines before the last line. These forms also work in ranges, gaps, and block addresses. |

## Operations

### PUT

Use a trailing colon for text:

```text
PUT <address>:
+first new line
+
+third new line
```

Every text row starts with `+`; `+` alone adds an empty line. A final newline in
the patch is allowed. Without the colon, `PUT` copies a register and cannot have
body rows:

```text
PUT >$ @saved
PUT >$
```

The latter uses the anonymous register from a bare `CUT`. A named register starts
with `@` and may contain only ASCII letters, digits, `_`, and `-`.

### CUT

```text
CUT <address> [@name]
```

`CUT` removes the addressed content and saves it in the named register, or in the
anonymous register when the name is omitted.

### REM

```text
REM
```

`REM` removes the entire section file, accepts no address, and cannot be combined
with another operation in that section.

### MV

```text
MV destination/path
```

`MV` moves the complete section file. The destination is one whitespace-free
path; matching single or double quotes are optional. It may appear once per
section and must come after any `PUT` or `CUT` operations. It also requires a
whole-file tagged read of the source.

## Example

```text
*** Begin Patch
[src/example.rs#CAFE]
PUT >25:
+inserted after line 25
CUT 13.=14 @fragment
MV src/renamed.rs
*** End Patch
```

If an edit returns a stale-tag or eligibility error, follow its steering text and
perform a new tagged read with the required rows or boundaries before retrying.
