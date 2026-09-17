# Standard library

Every module in `lib/`, with every declaration's signature and the comment above it. `lib/`
beside the compiler is on the module search path, so a module is used by qualifying the
name — `Json.parse`, `Json.JNum`, `Rand.Rng` — with no import. There is no visibility:
helpers are listed too, and callable, but the ones with a doc comment are the intended
surface.

Every module here passes `vibe check --prove`. Each has a runnable check that shows it in
use, `lib/check/<module>_check.vibe` (`vibe run` it).

Regenerate this file after changing `lib/`: each block is `vibe view --sig-only` on the module,
with the `;;` comment lines that precede each declaration in the source.

Idioms worth copying, all visible in the signatures below:

- a byte walk pins the length in a refinement, `(n:Size, n==len s) (i:Size)`, and measures
  `%(n - i)`;
- a parser whose position comes back from a callee threads `(fuel:U64)` and measures `%fuel`;
- a result that may fail on outside input is `Res Str t`, the `Str` saying where and why;
- a generator is a value handed back with each draw, `(U64, Rng)`.

## Args

```
;; Command-line arguments split by shape, with no declared schema: `--name=value`
;; is an option, `--name` and `-x` are flags, anything else is positional, and
;; everything after `--` is positional. Pass `drop 1 argv` to skip the program.
;; ponytail: no `--name value` form, which needs a schema to tell the value from a
;; positional; add one when a program wants it.
type Parsed = { flags:Vec Str, options:Vec (Str, Str), positional:Vec Str, rest:Bool }
;; One argument folded into what the ones before it produced.
take_arg (p:Parsed) (a:&Str) : Parsed
;; `name=value` split at the `=` found at `at`.
option_pair (body:&Str) (at:Size) : (Str, Str)
parse (args:&Vec Str) : Parsed
;; `--name` or `-name` given as a flag; `has_flag p "--verbose"`.
has_flag (p:&Parsed) (name:&Str) : Bool
;; The value of the last `--name=value`; `option p "out"`.
option (p:&Parsed) (name:&Str) : Opt Str
pick (found:Opt Str) (kv:&(Str, Str)) (name:&Str) : Opt Str
```

## Bits

```
;; U64 bit utilities. `band bor bxor bnot shl shr` are prelude functions and are
;; not repeated here. Bit indices are refined `< 64` so no shift loses a bit.
test_bit (x:U64) (i:Size, i<64) : Bool
set_bit (x:U64) (i:Size, i<64) : U64
clear_bit (x:U64) (i:Size, i<64) : U64
toggle_bit (x:U64) (i:Size, i<64) : U64
;; Counts set bits from index `k` upward; `acc<=k` keeps the sum in range.
popcount_from (x:U64) (k:U64, k<=64) (acc:U64, acc<=k) : U64
popcount (x:U64) : U64
is_pow2 (x:U64) : Bool
;; Isolates the lowest set bit; 0 for 0.
lowest_set (x:U64) : U64
rotl (x:U64) (n:Size, n<64) : U64
rotr (x:U64) (n:Size, n<64) : U64
;; The low `n` bits set.
low_mask (n:Size, n<=64) : U64
;; Index of the first set bit at or above `k`; 64 if there is none.
first_set_from (x:U64) (k:U64, k<=64) : U64
;; Number of zero bits below the lowest set bit; 64 for 0.
trailing_zeros (x:U64) : U64
;; Zeros above the highest set bit among the low `k` bits, counted from bit 63.
leading_from (x:U64) (k:U64, k<=64) : U64
;; Number of zero bits above the highest set bit; 64 for 0.
leading_zeros (x:U64) : U64
;; True when an odd number of bits are set.
parity (x:U64) : Bool
;; Byte `i`, little-endian: byte 0 is the lowest.
byte_at (x:U64) (i:Size, i<8) : U8
;; Combines up to eight bytes, lowest first, into a U64.
from_bytes_from (bs:&Vec U8) (k:Size, k<=8) (acc:U64) : U64
from_bytes (bs:&Vec U8) : U64
exp c test_bit, set_bit, clear_bit, toggle_bit, popcount, is_pow2, lowest_set, rotl, rotr, low_mask, trailing_zeros, leading_zeros, parity, byte_at
```

## Csv

```
;; RFC 4180 CSV: fields separated by `,`, records by `\n` or `\r\n`, a field in
;; double quotes may hold any of those and writes a quote as `""`. Every record
;; is a `Vec Str`; nothing checks that they are the same width.
;; The index just past an unquoted field: the next `,`, `\n`, `\r`, or the end.
field_end (s:&Str) (n:Size, n==len s) (i:Size) : Size
;; Inside a quoted field at `i`: the index of the closing quote, if there is one.
quote_end (s:&Str) (n:Size, n==len s) (i:Size) : Opt Size
;; The text of `s` in [a, b), "" when the range is empty.
cut (s:&Str) (a:Size) (b:Size) : Str
;; What follows a field ending at `k`: another field, a new record, or the end.
next (s:&Str) (n:Size, n==len s) (i:Size) (k:Size) (field:Str) (row:Vec Str) (rows:Vec (Vec Str)) (fuel:U64) : Res Str (Vec (Vec Str))
;; A new record at `i`; a line end as the last byte starts none.
record (s:&Str) (n:Size, n==len s) (i:Size) (rows:Vec (Vec Str)) (fuel:U64) : Res Str (Vec (Vec Str))
;; The field starting at `i` of the record `row`.
go (s:&Str) (n:Size, n==len s) (i:Size) (row:Vec Str) (rows:Vec (Vec Str)) (fuel:U64) : Res Str (Vec (Vec Str))
;; Every record of `s`. An empty input has none.
parse (s:&Str) : Res Str (Vec (Vec Str))
;; A field in quotes when it holds a comma, a quote or a line end.
field (f:&Str) : Str
row (r:&Vec Str) : Str
;; Records joined by `\n`, each one ended by it.
render (rows:&Vec (Vec Str)) : Str
```

## DictX

```
;; Helpers over the prelude's `Dict k v`, a hash map in insertion order.
get_or (d:&Dict k v) (key:&k) (fallback:v) : v
has_key (d:&Dict k v) (key:&k) : Bool
;; Applies `f` to the value at `key`, or to `fallback` when the key is absent.
update_with (f:v -> v) (d:Dict k v) (key:k) (fallback:v) : Dict k v
;; A tuple pattern on a borrow reads the fields, it does not take them, hence the dups.
insert_pair (d:Dict k v) (p:&(k, v)) : Dict k v
;; A later pair replaces an earlier one with the same key.
from_pairs (ps:&Vec (k, v)) : Dict k v
;; One step of `to_pairs`: the pair for `key`, if it is still there.
push_pair (d:&Dict k v) (acc:Vec (k, v)) (key:&k) : Vec (k, v)
to_pairs (d:&Dict k v) : Vec (k, v)
push_value (d:&Dict k v) (acc:Vec v) (key:&k) : Vec v
values (d:&Dict k v) : Vec v
;; Copies the entry at `key` from `src` into `acc`, overwriting.
copy_entry (src:&Dict k v) (acc:Dict k v) (key:&k) : Dict k v
map_entry (f:v -> w) (src:&Dict k v) (acc:Dict k w) (key:&k) : Dict k w
map_values (f:v -> w) (d:&Dict k v) : Dict k w
;; Right-biased: on a shared key the value from `b` wins.
merge (a:Dict k v) (b:&Dict k v) : Dict k v
;; Saturates at the U64 maximum rather than overflowing.
bump (d:Dict k U64) (key:k) : Dict k U64
;; How many elements map to each key under `f`.
count_by (f:a -> k) (xs:&Vec a) : Dict k U64
;; How many times each element occurs.
tally (xs:&Vec k) : Dict k U64
```

## Encoding

```
;; Hex and base64 (RFC 4648, standard alphabet, padded) over the bytes of a Str.
;; ponytail: output grows by `concat`, O(n²) on a long input; a byte buffer in
;; the prelude is the upgrade.
;; The character at `k` of `table`, "" past its end.
char_of (table:&Str) (k:U64) : Str
;; A nibble's value, 16 when the byte is not a hex digit.
nibble (b:U8) : U64
hex_go (s:&Str) (n:Size, n==len s) (i:Size) (acc:Str) : Str
;; Two lower-case hex digits per byte.
hex (s:&Str) : Str
unhex_go (s:&Str) (n:Size, n==len s) (i:Size) (acc:Str) : Res Str Str
;; The bytes two hex digits each stand for; either case is accepted.
unhex (s:&Str) : Res Str Str
;; The sextet of `v` that starts at bit `at`, as its base64 character.
sextet (v:U64) (at:Size) : Str
base64_go (s:&Str) (n:Size, n==len s) (i:Size) (acc:Str) : Str
;; Base64 with `=` padding.
base64 (s:&Str) : Str
;; A base64 character's value; 64 for `=`, 65 for anything else.
value64 (b:U8) : U64
unbase64_go (s:&Str) (n:Size, n==len s) (i:Size) (acc:Str) : Res Str Str
;; The bytes of padded base64 text.
unbase64 (s:&Str) : Res Str Str
```

## Hash

```
;; Non-cryptographic hashes: fast, stable across runs and machines, and no
;; defence against an adversary choosing the input.
fnv_go (s:&Str) (n:Size, n==len s) (i:Size) (h:U64) : U64
;; FNV-1a, 64-bit, over the bytes of `s`.
fnv1a (s:&Str) : U64
;; The splitmix64 finaliser: every bit of `x` moves every bit of the result.
mix (x:U64) : U64
;; `h` and `x` folded into one hash; the order matters.
combine (h:U64) (x:U64) : U64
```

## Io

```
;; `out` and `warn` already end the line, so there is no println / eprintln
;; here: they would be `out` and `warn` under another name.
ext c "stdlib.h"
;; `access` answers 0 when the path exists; `F_OK` is 0.
ext c "stdlib.h"
;; `getenv` may hand back NULL and a CStr cannot be compared with anything, so
;; `mblen` stands in as the NULL test: it answers 0 for NULL and for "".
ext c "stdlib.h"
;; Stdout without the newline `out` adds. Stops at a NUL byte.
print (s:&Str) : E! Unit
;; ponytail: no eprint — `stderr` is a C macro, not a function, and `dprintf` is hidden under -std=c99.
;; One line each.
print_all (xs:&Vec Str) : E! Unit
;; A trailing newline does not make an empty last line.
read_lines (path:&Str) : E! Vec Str
;; Every line is terminated, the last one included.
join_lines (xs:&Vec Str) : Str
write_lines (path:&Str) (xs:&Vec Str) : E! Unit
file_exists (path:&Str) : E! Bool
;; Creates the file when it is missing.
append (path:&Str) (s:&Str) : E! Unit
;; `argv` without the program name. The `Unit` parameter is there because a
;; nullary declaration other than `main` compiles to an unapplied closure.
args (u:Unit) : E! Vec Str
arg_at (i:Size) : E! Opt Str
;; The message and a newline on stderr, then exit status 1.
die (msg:&Str) : E! Unit
;; An unset variable and one set to "" both read as None.
getenv_opt (name:&Str) : E! Opt Str
exp c print, file_exists, append, die
```

## Json

```
;; JSON values, a parser and a renderer. An object keeps its keys in source
;; order, as a vector of pairs; a number is an F64.
type Json = JNull | JBool Bool | JNum F64 | JStr Str | JArr (Vec Json) | JObj (Vec (Str, Json))
;; The byte at `i` as a one-byte string, "" past the end.
at (s:&Str) (i:Size) : Str
skip_ws (s:&Str) (n:Size, n==len s) (i:Size) : Size
;; Whether `word` occurs in `s` at `i`.
word_at (s:&Str) (i:Size) (word:&Str) : Bool
hex_value (c:&Str) : Size
;; The four hex digits at `i`, or `None` when one of them is not a hex digit.
hex4 (s:&Str) (i:Size, i<len s && 3<len s - i) : Opt Size
;; A continuation byte: the six bits of `cp` from bit `at`.
cont (cp:Size) (at:Size) : Str
;; The UTF-8 encoding of a code point below 0x110000.
utf8 (cp:Size) : Str
;; The `\uXXXX` at `i` that is not half of a surrogate pair.
single_escape (s:&Str) (i:Size, i<len s && 5<len s - i) : Res Str Str
;; Whether the `\uXXXX` at `i` is a high surrogate, the first half of a pair.
high_at (s:&Str) (i:Size, i<len s && 5<len s - i) : Bool
;; The surrogate pair `\uD8xx\uDCxx` at `i`.
pair_escape (s:&Str) (i:Size, i<len s && 11<len s - i) : Res Str Str
;; The escape `\e` as the text it stands for; `u` is handled by `str_go`.
simple_escape (e:&Str) : Opt Str
;; The string body starting at `i`, just past the opening quote, and the
;; position after the closing one.
;; ponytail: grows `acc` a byte at a time, O(n²) on a long string.
str_go (s:&Str) (n:Size, n==len s) (i:Size) (acc:Str) : Res Str (Str, Size)
is_num_byte (c:&Str) : Bool
num_end (s:&Str) (n:Size, n==len s) (i:Size) : Size
number (s:&Str) (i:Size) : Res Str (Json, Size)
;; `word` at `i` stands for `v`.
keyword (s:&Str) (i:Size) (word:&Str) (v:Json) : Res Str (Json, Size)
;; `fuel` bounds the depth of the call chain, which is what makes the three
;; mutually recursive parsers total; `parse` gives it more than any input needs.
value (s:&Str) (i:Size) (fuel:U64) : Res Str (Json, Size)
;; Past `[` or past a `,`: the next element, or `]` when there is none yet.
arr (s:&Str) (i:Size) (acc:Vec Json) (fuel:U64) : Res Str (Json, Size)
;; Past `{` or past a `,`: the next `"key": value`, or `}` when there is none yet.
obj (s:&Str) (i:Size) (acc:Vec (Str, Json)) (fuel:U64) : Res Str (Json, Size)
;; Past the `:` of `key`.
obj_value (s:&Str) (i:Size) (acc:Vec (Str, Json)) (key:Str) (fuel:U64) : Res Str (Json, Size)
;; The whole of `s` as one JSON value; anything but whitespace after it is an error.
parse (s:&Str) : Res Str Json
quote (s:&Str) : Str
hex_digit (d:Size) : Str
;; ponytail: one pass over the string per control byte; a byte loop if a long
;; string ever makes 32 passes show.
escape_control (acc:Str) (k:Size) : Str
render_go (j:&Json) (fuel:U64) : Str
render_pair (p:&(Str, Json)) (fuel:U64) : Str
;; Compact JSON text. A number prints in the fewest digits that read back as
;; the same F64; NaN and infinities have no JSON spelling and come out as C prints them.
render (j:&Json) : Str
;; The value under `key` in an object; the first one when a key repeats.
field (j:&Json) (key:&Str) : Opt Json
pick_field (found:Opt Json) (p:&(Str, Json)) (key:&Str) : Opt Json
;; The element at `i` of an array.
index (j:&Json) (i:Size) : Opt Json
as_num (j:&Json) : Opt F64
as_str (j:&Json) : Opt Str
as_bool (j:&Json) : Opt Bool
is_null (j:&Json) : Bool
```

## List

```
;; Vec utilities the prelude does not have. Everything borrows the vector and
;; hands back fresh values; partial ones carry a refinement instead of a check.
any (p:a -> Bool) (xs:&Vec a) : Bool
all (p:a -> Bool) (xs:&Vec a) : Bool
count (p:a -> Bool) (xs:&Vec a) : Size
contains (y:&a) (xs:&Vec a) : Bool
find (p:a -> Bool) (xs:&Vec a) : Opt a
position (p:a -> Bool) (xs:&Vec a) : Opt Size
;; ponytail: the `|_` arm is unreachable; the exhaustiveness checker does not
;; accept a tuple pattern of binders as total.
position_step (p:a -> Bool) (found:Opt Size) (pair:&(Size, a)) : Opt Size
first (xs:&Vec a, len xs>0) : a
last (xs:&Vec a, len xs>0) : a
first_opt (xs:&Vec a) : Opt a
last_opt (xs:&Vec a) : Opt a
sum_by (f:a -> U64) (xs:&Vec a) : U64
flatten (xss:&Vec (Vec a)) : Vec a
enumerate (xs:&Vec a) : Vec (Size, a)
repeat (n:Size) (x:&a) : Vec a
partition (p:a -> Bool) (xs:&Vec a) : (Vec a, Vec a)
take_while (p:a -> Bool) (xs:&Vec a) : Vec a
drop_while (p:a -> Bool) (xs:&Vec a) : Vec a
zip (xs:&Vec a) (ys:&Vec b) : Vec (a, b)
zip_from (xs:&Vec a) (ys:&Vec b) (i:Size) (n:Size, n<=len xs && n<=len ys) (acc:Vec (a, b)) : Vec (a, b)
chunks (n:Size, n>0) (xs:&Vec a) : Vec (Vec a)
;; Appends to the last chunk while it has room, else opens a new one.
chunk_step (n:Size) (acc:Vec (Vec a)) (x:&a) : Vec (Vec a)
windows (n:Size, n>0) (xs:&Vec a) : Vec (Vec a)
;; ponytail: the `|_` arm is unreachable, as in `position_step`.
window_at (n:Size) (xs:&Vec a) (pair:&(Size, a)) : Vec a
dedup (xs:&Vec a) : Vec a
;; Element parameters are borrows: taking one by value would free a slot the
;; vector still owns.
dedup_step (acc:Vec a) (x:&a) : Vec a
swap (xs:Vec a) (i:Size, i<len xs) (j:Size, j<len xs) : Vec a
get_or (fallback:a) (xs:&Vec a) (i:Size) : a
```

## Math

```
;; The language has no `%` operator: `%` starts a termination measure. This is it.
imod (a:U64) (b:U64, b!=0) : U64
;; Truncating remainder, so the sign follows `a`, as it does in C.
irem (a:I64, a>0 - 9223372036854775807) (b:I64, b>0 - 9223372036854775807, b!=0) : I64
;; The remainder of non-negative operands, negated when `neg`. The solver's division
;; floors, so the signed form goes through here to stay provable.
rem_signed (neg:Bool) (a:I64, a>=0) (b:I64, b>0) : I64
even (a:U64) : Bool
odd (a:U64) : Bool
clamp (lo:U64) (hi:U64, lo<=hi) (x:U64) : U64
clamp_f64 (lo:F64) (hi:F64, lo<=hi) (x:F64) : F64
;; -1, 0 or 1.
sign (x:I64) : I64
;; -1.0, 0.0 or 1.0; NaN gives 1.0.
sign_f64 (x:F64) : F64
abs_diff (a:U64) (b:U64) : U64
;; Rounds the quotient up instead of down.
ceil_div (a:U64) (b:U64, b!=0) : U64
gcd (a:U64) (b:U64) : U64
;; Euclid on U64 takes at most 93 steps, so 100 fuel never runs out. The fuel is
;; there because the measure checker cannot learn `imod a b < b`.
gcd_fuel (a:U64) (b:U64) (fuel:U64) : U64
;; `lcm 0 x` is 0; `Er Overflow` when the result does not fit.
lcm (a:U64) (b:U64) : Res Fault U64
lcm_step (b:U64) (q:Res Fault U64) : Res Fault U64
;; Base-10 digit count; `digits 0` is 1.
digits (n:U64) : U64
;; A U64 has at most 20 digits, so the fuel never runs out; it stands in for the
;; measure `n / 10 < n`, which the checker cannot see.
digits_fuel (n:U64) (fuel:U64, fuel<=20) (acc:U64, acc + fuel<=20) : U64
;; `base` to the `power`, by squaring; `Er Overflow` when it does not fit.
pow_u64 (base:U64) (power:U64) : Res Fault U64
;; Halving a U64 reaches 0 within 64 steps, so the fuel never runs out.
pow_fuel (base:U64) (power:U64) (fuel:U64) : Res Fault U64
pow_square (base:U64) (power:U64) (half:Res Fault U64) : Res Fault U64
;; 20! is the largest factorial that fits in a U64. A table, because the solver
;; cannot bound `n * factorial (n - 1)` through the recursive call.
factorial (n:U64, n<=20) : U64
lerp (a:F64) (b:F64) (t:F64) : F64
ceil (x:F64) : F64
;; Half away from zero.
round (x:F64) : F64
trunc (x:F64) : F64
;; `lcm` and `pow_u64` return `Res`, which cannot cross the C boundary.
exp c imod, irem, even, odd, clamp, clamp_f64, sign, sign_f64, abs_diff, ceil_div, gcd, digits, factorial, lerp, ceil, round, trunc
```

## Opt

```
;; Combinators for the prelude `Opt t = Some t | None`.
;;
;; Every function borrows its `Opt` (or `Res`) and copies the payload it hands
;; on with `dup`. ponytail: the natural signatures consume the option, but the
;; compiler drops a matched owned scrutinee deeply after its payload has been
;; moved out of an arm (`?o |Some v -> v` frees `v` under the caller: a
;; use-after-free). Upgrade path: once a moved-out payload suppresses the
;; scrutinee's drop, take `Opt a` by value and remove the `dup`s.
is_some (o:&Opt a) : Bool
is_none (o:&Opt a) : Bool
;; The payload, or `fallback`.
unwrap_or (fallback:a) (o:&Opt a) : a
map (f:a -> b) (o:&Opt a) : Opt b
;; Chain a step that may itself produce nothing.
and_then (f:a -> Opt b) (o:&Opt a) : Opt b
;; `o` if it holds a value, otherwise `alt`.
or_else (alt:Opt a) (o:&Opt a) : Opt a
;; Keep the payload only when `keep` accepts it.
filter (keep:&a -> Bool) (o:&Opt a) : Opt a
;; `Ok` of the payload, or `Er err` when there is none.
to_res (err:e) (o:&Opt a) : Res e a
;; The `Ok` payload; an error becomes `None`.
from_res (r:&Res e a) : Opt a
from_bool (cond:Bool) (x:a) : Opt a
;; Both payloads as a pair, or `None` if either is missing.
zip (oa:&Opt a) (ob:&Opt b) : Opt (a, b)
flatten (o:&Opt (Opt a)) : Opt a
;; ponytail: `values` takes the vector by value only because the borrow checker
;; treats any `fold` result over a borrowed vector as escaping it, even through
;; `dup`; the payloads are still copied, since moving them out double-frees.
values (xs:Vec (Opt a)) : Vec a
count_some (xs:&Vec (Opt a)) : Size
```

## Path

```
;; POSIX paths as text: `/` separates, nothing touches the file system.
is_absolute (p:&Str) : Bool
;; The non-empty parts between slashes.
segments (p:&Str) : Vec Str
;; `b` under `a`; an absolute `b` stands on its own.
join (a:&Str) (b:&Str) : Str
;; The last segment; "/" for the root and "" for an empty path.
basename (p:&Str) : Str
;; Everything before the last segment; "." when that is nothing.
dirname (p:&Str) : Str
;; The text after the last `.` of the basename, "" when there is none. A
;; leading dot names a hidden file rather than starting an extension.
extension (p:&Str) : Str
;; The basename without its extension.
stem (p:&Str) : Str
;; One segment applied to the ones kept so far.
step (absolute:Bool) (kept:Vec Str) (seg:&Str) : Vec Str
;; `.` and `..` resolved lexically and repeated slashes folded: `a/./b/../c` is
;; `a/c`. A `..` above the root stays at the root.
normalize (p:&Str) : Str
```

## Rand

```
;; A seeded pseudo-random generator, splitmix64. Deterministic for a seed and
;; not for secrets. The generator is a value: every draw hands back the next one.
type Rng = { state:U64 }
seed (n:U64) : Rng
;; A 64-bit draw and the generator after it.
next (r:Rng) : (U64, Rng)
;; A draw in [0, bound). ponytail: the remainder of a plain division, so a bound
;; far from a power of two is slightly biased; rejection sampling if that matters.
below (r:Rng) (bound:U64, bound>0) : (U64, Rng)
;; A draw in [0, 1) with 53 bits of precision.
unit (r:Rng) : (F64, Rng)
```

## Res

```
;; Combinators for the prelude `Res e t = Ok t | Er e`.
;;
;; Every function borrows its `Res` (or `Opt`) and copies the payload it hands
;; on with `dup`. ponytail: the natural signatures consume the result, but the
;; compiler drops a matched owned scrutinee deeply after its payload has been
;; moved out of an arm (`?r |Ok v -> v` frees `v` under the caller: a
;; use-after-free). Upgrade path: once a moved-out payload suppresses the
;; scrutinee's drop, take `Res e t` by value and remove the `dup`s.
is_ok (r:&Res e t) : Bool
is_er (r:&Res e t) : Bool
;; The `Ok` payload, or `fallback`.
unwrap_or (fallback:t) (r:&Res e t) : t
;; The `Ok` payload, or `handle` applied to the error.
unwrap_or_else (handle:e -> t) (r:&Res e t) : t
map (f:t -> u) (r:&Res e t) : Res e u
map_err (f:e -> d) (r:&Res e t) : Res d t
;; Chain a step that can itself fail; the first error wins.
and_then (f:t -> Res e u) (r:&Res e t) : Res e u
;; Recover from an error with a step that can itself fail.
or_else (f:e -> Res d t) (r:&Res e t) : Res d t
to_opt (r:&Res e t) : Opt t
err_opt (r:&Res e t) : Opt e
;; `Some v` is `Ok v`; `None` is `Er err`.
from_opt (err:e) (o:&Opt t) : Res e t
;; Fold steps for `oks` / `ers`. Named, not lambdas: the escape check reads a
;; lambda whose tail is its accumulator as handing out an element of the vector.
push_ok (acc:Vec t) (r:&Res e t) : Vec t
push_er (acc:Vec e) (r:&Res e t) : Vec e
;; The `Ok` payloads, in order.
oks (rs:&Vec (Res e t)) : Vec t
;; The `Er` payloads, in order.
ers (rs:&Vec (Res e t)) : Vec e
;; How many results are `Ok`.
count_ok (rs:&Vec (Res e t)) : Size
fault_name (f:&Fault) : Str
```

## Search

```
;; First index in [lo, hi) whose value is not below `x`; `hi` when there is none.
;; Each step halves `hi - lo`, so 64 fuel never runs out. The fuel is there because
;; the measure checker cannot see `(hi - lo) / 2 < hi - lo`. Every `get` in this
;; module is proved in bounds: no `get_checked`, no run-time index assert.
lower_from (v:&Vec U64) (x:U64) (lo:Size) (hi:Size, lo<=hi && hi<=len v) (fuel:U64) : Size
;; First index in [lo, hi) whose value is above `x`; `hi` when there is none.
upper_from (v:&Vec U64) (x:U64) (lo:Size) (hi:Size, lo<=hi && hi<=len v) (fuel:U64) : Size
lower_n (v:&Vec U64) (x:U64) (n:Size, n==len v) : Size
upper_n (v:&Vec U64) (x:U64) (n:Size, n==len v) : Size
;; On a sorted `v`: the first index whose value is `>= x`, `len v` when none.
lower_bound (v:&Vec U64) (x:U64) : Size
;; On a sorted `v`: the first index whose value is `> x`, `len v` when none.
upper_bound (v:&Vec U64) (x:U64) : Size
;; `Some i` when `i` is in bounds and holds `x`.
found_at (v:&Vec U64) (x:U64) (i:Size) : Opt Size
;; On a sorted `v`: the index of the first `x`, if any.
binary_search (v:&Vec U64) (x:U64) : Opt Size
;; `lower_from` for a vector sorted by `key`.
lower_from_by (key:a -> I64) (v:&Vec a) (x:I64) (lo:Size) (hi:Size, lo<=hi && hi<=len v) (fuel:U64) : Size
lower_n_by (key:a -> I64) (v:&Vec a) (x:I64) (n:Size, n==len v) : Size
;; On a `v` sorted by `key`: the first index whose key is `>= x`, `len v` when none.
lower_bound_by (key:a -> I64) (v:&Vec a) (x:I64) : Size
;; `Some i` when `i` is in bounds and its key is `x`.
found_at_by (key:a -> I64) (v:&Vec a) (x:I64) (i:Size) : Opt Size
;; On a `v` sorted by `key`: the index of the first element whose key is `x`, if any.
binary_search_by (key:a -> I64) (v:&Vec a) (x:I64) : Opt Size
sorted_from (v:&Vec I64) (i:Size) (n:Size, n==len v) : Bool
;; Non-decreasing; the empty vector is sorted.
is_sorted (v:&Vec I64) : Bool
;; Inserts `x` after any equal elements, so a sorted `v` stays sorted.
insert_sorted (v:&Vec U64) (x:U64) : Vec U64
merge_from (a:&Vec U64) (b:&Vec U64) (i:Size) (na:Size, na==len a) (j:Size) (nb:Size, nb==len b) (acc:Vec U64) : Vec U64
;; Two sorted vectors into one; on ties the element of `a` comes first.
merge (a:&Vec U64) (b:&Vec U64) : Vec U64
unique_from (v:&Vec U64) (i:Size) (n:Size, n==len v) (last:U64) (acc:Vec U64) : Vec U64
;; A sorted `v` with each run of equal values kept once.
unique_sorted (v:&Vec U64) : Vec U64
;; The `k`th smallest value, counting from 0.
;; `sort_by` keeps the length, but no postcondition says so; the fallback arm is
;; unreachable, and still proved in bounds.
kth_smallest (v:&Vec U64) (k:Size, k<len v) : U64
min_from (v:&Vec I64) (i:Size) (n:Size, n==len v) (best:Size, best<n) : Size
max_from (v:&Vec I64) (i:Size) (n:Size, n==len v) (best:Size, best<n) : Size
;; Index of the first smallest element.
min_index (v:&Vec I64, len v>0) : Size
;; Index of the first largest element.
max_index (v:&Vec I64, len v>0) : Size
```

## Set

```
;; A set is a `Dict k Unit`, the prelude's own convention: the keys are the
;; elements and the value carries nothing. Lookups are O(1).
from_vec (xs:&Vec k) : Dict k Unit
member (s:&Dict k Unit) (x:&k) : Bool
add (s:Dict k Unit) (x:k) : Dict k Unit
delete (s:Dict k Unit) (x:&k) : Dict k Unit
to_vec (s:&Dict k Unit) : Vec k
size (s:&Dict k Unit) : Size
union (a:Dict k Unit) (b:&Dict k Unit) : Dict k Unit
intersect (a:&Dict k Unit) (b:&Dict k Unit) : Dict k Unit
difference (a:&Dict k Unit) (b:&Dict k Unit) : Dict k Unit
is_subset (a:&Dict k Unit) (b:&Dict k Unit) : Bool
```

## Stats

```
;; Descriptive statistics over `&Vec F64`.
;; Every proof here is about mathematical reals, not IEEE floats: nothing is claimed
;; about rounding, overflow to infinity, or NaN.
mean (xs:&Vec F64, len xs>0) : F64
minimum (xs:&Vec F64, len xs>0) : F64
maximum (xs:&Vec F64, len xs>0) : F64
range_width (xs:&Vec F64, len xs>0) : F64
;; Sum of squared values.
sum_sq (xs:&Vec F64) : F64
;; Sum of squared deviations from the mean.
sq_dev (xs:&Vec F64, len xs>0) : F64
variance (xs:&Vec F64, len xs>0) : F64
sample_variance (xs:&Vec F64, len xs>1) : F64
stddev (xs:&Vec F64, len xs>0) : F64
sample_stddev (xs:&Vec F64, len xs>1) : F64
dot_step (xs:&Vec F64) (ys:&Vec F64) (acc:F64) (i:Size) : F64
dot (xs:&Vec F64) (ys:&Vec F64, len ys==len xs) : F64
;; ponytail: `sum ws>0.0` cannot be a refinement (a call in a refinement teaches the
;; body nothing), so zero total weight gives 0.0.
weighted_mean (xs:&Vec F64) (ws:&Vec F64, len ws==len xs) : F64
;; `sort_by` keeps the length, but no postcondition says so; the fallback arm is
;; unreachable.
at (v:&Vec F64) (i:Size) : F64
median (xs:&Vec F64, len xs>0) : F64
;; Lower nearest rank on the sorted data: index floor (p * (n - 1)).
percentile (p:F64, p>=0.0 && p<=1.0) (xs:&Vec F64, len xs>0) : F64
;; Min-max scaling into [0, 1].
;; ponytail: `maximum xs>minimum xs` cannot be a refinement the body can use; zero
;; spread maps every value to 0.0.
normalize (xs:&Vec F64, len xs>0) : Vec F64
;; How many standard deviations `x` lies from the mean; 0.0 when the spread is zero.
zscore (x:F64) (xs:&Vec F64, len xs>0) : F64
bucket_of (n:Size, n>0) (lo:F64) (w:F64, w>0.0) (x:F64) : Size
;; Counts per bucket of `n` equal-width buckets over [minimum, maximum]; the maximum
;; lands in the last bucket, and zero spread puts everything in the first.
histogram (n:Size, n>0) (xs:&Vec F64, len xs>0) : Vec Size
exp c mean, median, variance, sample_variance, stddev, sample_stddev, minimum, maximum, range_width, sum_sq, dot, weighted_mean, percentile, zscore
```

## Text

```
;; String and character utilities the prelude does not have. Strings are bytes:
;; `len`, `slice` and everything here count bytes, so the classification is
;; ASCII only. The haystack comes first, as it does in `contains` and `replace`.
is_digit (c:Char) : Bool
is_upper (c:Char) : Bool
is_lower (c:Char) : Bool
is_alpha (c:Char) : Bool
is_alnum (c:Char) : Bool
;; Space, tab, newline, vertical tab, form feed, carriage return.
is_space (c:Char) : Bool
;; `Some 7` for '7'; `None` for anything that is not a decimal digit.
digit_value (c:Char) : Opt U32
is_empty (s:&Str) : Bool
ends_with (s:&Str) (suffix:&Str) : Bool
strip_prefix (s:&Str) (prefix:&Str) : Opt Str
strip_suffix (s:&Str) (suffix:&Str) : Opt Str
;; `sep` between every two parts; `join ", " &[]` is "".
join (sep:&Str) (parts:&Vec Str) : Str
unlines (ls:&Vec Str) : Str
unwords (ws:&Vec Str) : Str
repeat (s:&Str) (n:Size) : Str
;; Pads with `fill` on the left up to `width` bytes; a longer string is unchanged.
pad_left (s:&Str) (width:Size) (fill:Char) : Str
pad_right (s:&Str) (width:Size) (fill:Char) : Str
;; Splits on runs of space, tab, newline and carriage return; no empty words.
;; ponytail: no `\v`/`\f` — the lexer has no escape for them.
words (s:&Str) : Vec Str
;; Non-overlapping occurrences of `needle`; an empty needle occurs 0 times.
count (s:&Str) (needle:&Str) : Size
chars_go (s:&Str) (rest:Size, rest<=len s) (acc:Vec Str) : Vec Str
;; One single-byte string per byte.
chars (s:&Str) : Vec Str
reverse (s:&Str) : Str
;; `pair` is a lower-case byte followed by its upper-case form.
upper_one (acc:Str) (pair:&Str) : Str
;; ASCII only, like `lower`.
upper (s:&Str) : Str
;; First byte upper-cased, the rest untouched.
capitalize (s:&Str) : Str
exp c is_digit, is_upper, is_lower, is_alpha, is_alnum, is_space, is_empty, ends_with, repeat, pad_left, pad_right, count, reverse, upper, capitalize
```

## Time

```
;; A calendar date, proleptic Gregorian. Years 1..9999 are the supported range:
;; it keeps every intermediate of the day arithmetic non-negative (so C's
;; truncating division agrees with the solver's flooring one) and in range.
type Date = { year:I64, month:I64, day:I64, year>=1, year<=9999, month>=1, month<=12, day>=1, day<=31 }
;; `time` accepts a pointer it writes the result through. There is no NULL
;; `Ptr` in the language, so a scratch cell is allocated and freed.
ext c "unistd.h"
ext c "unistd.h"
;; ponytail: no monotonic clock; a nullary `ext c` (`clock`) is a closure, never called, and `Unit` cannot cross the boundary.
;; ponytail: no `sleep_ms`; `usleep` is hidden under -std=c11 and `nanosleep` needs a struct.
ext c "unistd.h"
;; Seconds since 1970-01-01T00:00:00Z.
now_unix (u:Unit) : E! I64
;; Sleeps whole seconds; answers the seconds left if a signal cut it short.
sleep_s (secs:U32) : E! U32
is_leap_year (year:I64, year>=1, year<=9999) : Bool
days_in_month (year:I64, year>=1, year<=9999) (month:I64, month>=1, month<=12) : I64
;; Days since 1970-01-01 (negative before it). Hinnant's `days_from_civil`.
days_from_civil (d:&Date) : I64
;; The core of `days_from_civil`, on a year that starts in March: `mp` 0 is March.
civil_days (y:I64, y>=0, y<=9999) (mp:I64, mp>=0, mp<=11) (day:I64, day>=1, day<=31) : I64
;; The date `days` after 1970-01-01. Hinnant's `civil_from_days`; the bounds are
;; 0001-01-01 and 9999-12-31.
civil_from_days (days:I64, days>=0 - 719162, days<=2932896) : Date
;; 0 = Sunday … 6 = Saturday, for a day count as `days_from_civil` gives.
weekday (days:I64, days>=0 - 719162, days<=2932896) : I64
;; Whole days since the epoch, rounded towards the past, of a Unix timestamp.
days_from_unix (secs:I64, secs>=0 - 62135596800, secs<=253402300799) : I64
pad2 (n:I64) : Str
pad4 (n:I64) : Str
;; "YYYY-MM-DDTHH:MM:SSZ" for a Unix timestamp in years 1..9999.
to_iso8601 (secs:I64, secs>=0 - 62135596800, secs<=253402300799) : Str
;; ponytail: `now_unix` is not exported; a `Unit` parameter emits an invalid C prototype.
exp c is_leap_year, days_in_month, weekday, days_from_unix, to_iso8601, sleep_s
```

