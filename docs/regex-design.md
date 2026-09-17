# Regex — design

`lib/regex.vibe`, module `Regex`. RE2 syntax and leftmost-first semantics, the
way Go's `regexp` does them, executed by a Pike VM: a Thompson NFA simulated
with prioritised thread lists, so matching is linear in the input for every
pattern. The first version is Go's first version — one engine, a literal
prefix skip, anchors — and the faster layers (one-pass, bounded backtracker,
lazy DFA) come when a profile asks for them.

## API

Subject first, pattern second, as `contains`, `index_of` and `replace` are.
Errors are `Res Str`, as `Json.parse` and `Csv.parse` are. Positions are byte
offsets, as Go's are and as `slice` takes them.

```
type Regex                                              ;; compiled; opaque by convention

compile   : &Str -> Res Str Regex
matches   : &Str -> &Regex -> Bool
find      : &Str -> &Regex -> Opt (Size, Size)          ;; [start, end)
find_all  : &Str -> &Regex -> Vec (Size, Size)
captures  : &Str -> &Regex -> Opt (Vec (Opt Str))       ;; 0 is the whole match
group     : &Regex -> &Str -> Opt Size                  ;; a group's name to its index
replace   : &Str -> &Regex -> &Str -> Str               ;; every match; $1 ${1} $name ${name} $$
split     : &Str -> &Regex -> Vec Str
```

Semantics follow Go's, function by function:

- `find_all`: non-overlapping, left to right; an empty match that abuts the
  previous match is skipped, and after an empty match the search resumes one
  code point further on (`FindAllStringIndex(s, -1)`).
- `captures`: `None` for a group that did not take part in the match
  (`FindStringSubmatchIndex`'s `-1`).
- `replace`: the template language of `Regexp.Expand`. `$name` takes the
  longest run of letters, digits and `_`; a name or index that is not a group
  expands to nothing; `$$` is a `$`.
- `split`: `Split(s, -1)`.

## Syntax

Go's `regexp/syntax` with `Perl` flags, minus Unicode tables:

| | |
|---|---|
| literals | any code point; escapes `\a \f \t \n \r \v \x7F \x{10FFFF}`, octal `\123`, `\Q…\E`, `\` before punctuation |
| any | `.` (not `\n` unless `s`) |
| classes | `[abc] [^abc] [a-z]`, nested escapes, `[[:alpha:]]` and the other 13 POSIX names, ASCII |
| perl classes | `\d \D \w \W \s \S`, ASCII, as in Go |
| anchors | `^ $` (line-wise under `m`), `\A \z`, `\b \B` (ASCII word) |
| groups | `(re)`, `(?:re)`, `(?P<name>re)`, `(?<name>re)` |
| flags | `(?flags)` and `(?flags:re)` with `i m s U` and `-` |
| repetition | `* + ? {n} {n,} {n,m}` and their lazy `?` forms; bound ≤ 1000 |
| alternation | `a\|b`, leftmost-first |

Refused with a message naming the offset: `\p{…}` and `\pL` ("Unicode classes
are not supported yet"), backreferences, lookaround, a repeat over 1000, nesting
deeper than 1000, invalid UTF-8 in the pattern. Case folding under `i` is ASCII.

The subject is decoded as UTF-8 one code point at a time; an invalid byte is
U+FFFD with width 1, as in Go.

## Structure

One file, four stages, each a handful of functions with one job.

1. **Parse** — recursive descent over the pattern's bytes into
   `Node = Empty | Lit U32 | Class (Vec (U32, U32)) | Look Look | Cat (Vec Node)
   | Alt (Vec Node) | Rep Node Size Size Bool | Cap Node Size`.
   Flags are applied while parsing: `i` widens a literal or a class to both
   cases, `s` decides what `.` is, `m` decides which assertion `^` and `$` are,
   `U` flips greediness. A class is a sorted, merged vector of code-point
   ranges, negation already applied. Names go into a `Vec Str`, index-aligned
   with the groups.
2. **Compile** — `Node` to `Vec Inst` with
   `Inst = Match | Range (Vec (U32, U32)) Size | Split Size Size | Jmp Size
   | Save Size Size | Assert Look Size | Fail`.
   `Split`'s first target has priority. `{n,m}` expands to copies, which is why
   its bound is capped. The compiler also records the literal prefix every
   match must start with, and whether the program is anchored at `\A`.
3. **Run** — the Pike VM. Two thread lists, each a sparse set over program
   counters plus a flat `Vec Size` of capture slots (`pc * slots`, `NONE` as
   the unset sentinel). `add` follows `Jmp`, `Split`, `Save` and `Assert` to
   their closure with an explicit stack, so no program counter is added twice
   in one step. Stepping a `Range` thread consumes one code point; reaching
   `Match` records the slots and drops every lower-priority thread. A new
   thread starts at each position until a match is found, unless anchored;
   when the list is empty the scan jumps with `index_of` to the next place
   the literal prefix occurs.
4. **API** — the eight functions above, over `run`.

Every recursion is total: the parser and `add` carry `fuel` bounded by the
pattern length and the program length, the scan's measure is `n - i`.

## Verification

- `lib/check/regex_check.vibe`, like every other module: a table of pattern,
  subject and expected result per function, including the Go semantics that are
  easy to get wrong (empty matches in `find_all`, `split` on an empty match,
  non-participating groups, lazy repeats, `\b` at the ends).
- A differential test against Go: a small Go program and the check agree on
  `find_all` and `captures` for a corpus of patterns and subjects; the corpus
  and the Go program live in `lib/check/regex_go/`, and the comparison is run
  by hand, not by `cargo test`, which should not need Go.
- `vibe check --prove lib/regex.vibe`, as every module in `lib/` passes.
- Linear time: a pathological pattern (`(a*)*b` over 100,000 `a`s) finishes
  in time proportional to the input.

## Out of scope for this version

Unicode tables (`\p{…}`, Unicode case folding), leftmost-longest (`Longest`),
one-pass and backtracking engines, a lazy DFA, `replace` with a function.

## Measurement

The tokens spent writing the library — code, check, differential test, docs of
the module — are measured from the session log: the usage of every model
response between the first file written for it and its last commit, nothing
before or after. The figure goes in the README.
