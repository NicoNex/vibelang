# Prelude, grammar and operators

Contents:
1. [Grammar in brief](#grammar-in-brief)
2. [Literals and comments](#literals-and-comments)
3. [Operators](#operators)
4. [Types](#types)
5. [The three specials: `len`, `show`, `fmt`](#the-three-specials)
6. [Every prelude signature](#every-prelude-signature)
7. [What the prelude does not have](#what-the-prelude-does-not-have)

The signature list is transcribed from `src/types.rs::PRELUDE_SIGS`, which is the
compiler's own table. If it disagrees with this file, the table wins — re-read it.

---

## Grammar in brief

```ebnf
module      = "mod" ModName NL { decl } ;
decl        = typedecl | fundecl | extblock | expdecl | ghostdecl ;

typedecl    = "type" TypeName "=" typebody ;
typebody    = record | variants ;
record      = "{" field { "," field } [ "," refine ] "}" ;
variants    = variant { "|" variant } ;
variant     = CtorName { type } ;

fundecl     = name { param } [ ":" type ] "=" expr [ measure ] ;
param       = name | "(" name { name } ":" type [ "," refine ] ")" ;
measure     = "%" expr ;

expr        = app | match | bind | letexpr | lambda | literal | record
            | expr binop expr | expr "|>" expr ;
app         = atom { atom } ;              (* juxtaposition, curried *)
match       = "?" expr arm { arm } "end" ;
arm         = "|" pattern "->" expr ;
bind        = name "<-" expr ";" expr ;    (* only in an E! body *)
seq         = expr ";" expr ;              (* `a ; b` is `_ <- a ; b` *)
letexpr     = "let" name "=" expr "in" expr ;
lambda      = "\" name { name } "->" expr ;
arena       = "arena" name "in" expr ;

pattern     = literal | name | "_" | CtorName { pattern }
            | "[" [ pattern { "," pattern } ] "]"
            | "(" pattern { "," pattern } ")" ;

type        = TypeName { type } | type "->" type | "&" type | "E!" type
            | "(" type { "," type } ")" ;

extblock    = "ext" "c" StringLit { extsig } "end" ;
extsig      = name ":" type [ "," refine ] NL ;
expdecl     = "exp" "c" name { "," name } ;
```

Several params of the same type share one group: `(a b:U64)` binds both.
A refinement attaches to the group: `(k:Nat) (n:Nat, k<=n)`.

**Layout.** Indentation is not significant. Blank lines are free. The one rule left: a
newline ends a top-level declaration, so `f : U64 = 1` and `g : U64 = 2` may not share a
line. It applies only outside every bracket and every `end`, only when the tokens so far
already form an expression, and only if the next token could begin one. The asymmetry that
follows: `-` is also unary negation, so `a\n- b` ends the declaration while `a -\nb` does
not.

**Canonicity.** The parser *rejects* rather than normalises: redundant parentheses around
an atom (`(n)`, `(1)`, `(Foo)`) are `canon.parens`; a `match` or `ext c` without `end` is
`parse.match` / `parse.ext`; a `<-` not terminated by `;` is `parse.bind`. `vibe fmt` then
settles the layout — including where long argument lists wrap — and `vibe fmt --check`
fails on anything that is not already what it would write.

Reserved words (cannot be identifiers): `mod ext exp type ghost let in own ref arena with
end True False`. `own` and `ref` are reserved for `exp c` ownership qualifiers that the
spec describes but the compiler does not yet act on. `c` is a keyword only immediately
after `ext` / `exp`; elsewhere it is an ordinary name.

---

## Literals and comments

| form | example |
|---|---|
| comment | `;; to end of line` |
| integer | `42`, `0` |
| float | `1.5`, `0.0` — a literal without a point adopts whatever numeric type surrounds it, one with a point constrains to `F32`/`F64` |
| string | `"hi\n"` |
| char | `'A'` |
| bool | `True`, `False` |
| unit | `()` |
| list | `[1, 2, 3]` — a `Vec a`, and also a pattern |
| tuple | `(1, dup "a")` — and also a pattern |
| record | `{sku=s, qty=n}` |
| record update | `{t with qty=12}` |
| borrow | `&x`, `&(f y)`, `&{a=1.0, b=4.0}` |
| lambda | `\x -> x * 2`, `\a b -> a + b` |
| arena | `arena a in len (rev &(range 0 n))` |

`;;` is a comment and `;` is the sequencer; the lexer takes the longer token first, so a
trailing comment after a `;` is still a comment.

---

## Operators

Precedence, loosest first. There is no user-defined operator and no overloading.

| prec | operators | meaning |
|---|---|---|
| — | `\|>` | pipe forward: `xs \|> f a` is `f a xs` (feeds the **last** argument) |
| 1 | `\|\|` | boolean or |
| 2 | `&&` | boolean and |
| 3 | `== != < <= > >=` | comparison, result `Bool` |
| 4 | `++` | `Str` concatenation (`&Str` operands, `Str` result) |
| 5 | `+ -` | numeric |
| 6 | `* /` | numeric |
| unary | `-` | negation |
| unary | `!` | boolean not |
| postfix | `.` | field access; must not be preceded by a space |

**There is no modulo operator.** `%` starts a termination measure, and `f a b = a % b`
parses silently as body `a` with measure `b`. Check with `vibe view` if in doubt.

Bit operations are prelude **functions** — `band bor bxor bnot shl shr ord` — because `&`
is the borrow sigil, `|` separates match arms, and a call carries no precedence to get
wrong.

---

## Types

Scalars — **copied, not moved**: every `U8…U64`, `I8…I64`, `F32`, `F64`, `Bool`, `Char`,
`Unit`, `Nat`, `Size`, `CStr`, `Ptr`.

Affine — **moved**: `Str`, `Vec a`, records, ADTs, tuples containing any of these.

`Nat` and `Size` normalise to `U64` in codegen and at the C boundary. `Nat` reads as a
mathematical natural in a refinement.

`&T` is a borrow valid for the duration of the call. It is erased in the type checker —
affine *use* is what `own.rs` enforces — so `&` is about who frees, not about a distinct
type.

`E! T` is a computation producing `T` while touching the world. `E!` scopes over the whole
rest of the type: `load (p:&Str) : E! Res Err (Vec Tx)`.

Prelude data types:

```
type Res e t = Ok t | Er e
type Opt t   = Some t | None
type Fault   = Overflow | DivZero | OutOfBounds | BadParse
```

---

## The three specials

Typed by a rule, not by a signature:

| name | type | note |
|---|---|---|
| `len` | `a -> U64` | accepts only `Vec a` and `Str`; anything else is `type.mismatch` |
| `show` | `a -> Str` | any value to a string |
| `fmt` | `Str -> … -> Str` | variadic; `{}` is the hole. `fmt "n={} tot={}" (len ts) t` |

---

## Every prelude signature

### Copy

```
dup          : &a -> a
```

Deep and generic: the copy owns everything it points at, so it survives the original and is
freed on its own. `CStr` and `Ptr` are the exception — they stay shallow, because C owns
that memory, so a copy holding one still aliases.

### Vec

```
empty        : Vec a
single       : a -> Vec a
push         : Vec a -> a -> Vec a
get          : &Vec a -> Size -> a
set          : Vec a -> Size -> a -> Vec a
map          : (a -> b) -> &Vec a -> Vec b
filter       : (a -> Bool) -> &Vec a -> Vec a
fold         : (b -> a -> b) -> b -> &Vec a -> b
each         : (a -> E! Unit) -> &Vec a -> E! Unit
sum          : &Vec a -> a
max_by       : (a -> b) -> &Vec a -> a
min_by       : (a -> b) -> &Vec a -> a
sort_by      : (a -> b) -> &Vec a -> Vec a
rev          : &Vec a -> Vec a
concat_vec   : &Vec a -> &Vec a -> Vec a
seq          : Vec (Res e t) -> Res e (Vec t)
range        : Size -> Size -> Vec Size
take         : Size -> &Vec a -> Vec a
drop         : Size -> &Vec a -> Vec a
```

`get v i` carries the only hardcoded builtin refinement: `i < len v`.

### Str

```
split        : Char -> &Str -> Vec Str
lines        : &Str -> Vec Str
concat       : &Str -> &Str -> Str
trim         : &Str -> Str
starts_with  : &Str -> &Str -> Bool
contains     : &Str -> &Str -> Bool
to_cstr      : &Str -> CStr
from_cstr    : CStr -> Str
chr          : Char -> Str
slice        : Size -> Size -> &Str -> Opt Str
index_of     : &Str -> &Str -> Opt Size
replace      : &Str -> &Str -> &Str -> Str
lower        : &Str -> Str
```

`slice` and `index_of` return `Opt` because the bootstrap cannot yet phrase their
refinements on a prelude name — not because `Opt` was the better design.

### Conversions and parsing

No overloading: one name, one meaning.

```
f32 f64      : a -> F32 / F64
i8 i16 i32 i64 : a -> I8 / I16 / I32 / I64
u8 u16 u32 u64 : a -> U8 / U16 / U32 / U64
size         : a -> Size
parse_i64    : &Str -> Opt I64
parse_u32    : &Str -> Opt U32
parse_u64    : &Str -> Opt U64
parse_f64    : &Str -> Opt F64
```

### Bits

```
band         : a -> a -> a
bor          : a -> a -> a
bxor         : a -> a -> a
bnot         : a -> a
shl          : a -> Size -> a
shr          : a -> Size -> a
ord          : Char -> U32
```

`shr` is arithmetic on a signed value and logical on an unsigned one — which is what the
value's own type already says.

### Math

```
abs          : a -> a
min          : a -> a -> a
max          : a -> a -> a
sqrt         : F64 -> F64
pow          : F64 -> F64 -> F64
floor        : F64 -> F64
```

### IO — all effectful

```
read         : &Str -> E! Str
write        : &Str -> &Str -> E! Unit
out          : &Str -> E! Unit
warn         : &Str -> E! Unit
argv         : E! Vec Str
read_stdin   : E! Str
exit         : I32 -> E! Unit
```

`argv` includes the program name at index 0; `drop 1 args` is the usual first step.

### Checked — moves the obligation to run time (spec §7.5)

```
add_checked  : a -> a -> Res Fault a
sub_checked  : a -> a -> Res Fault a
mul_checked  : a -> a -> Res Fault a
div_checked  : a -> a -> Res Fault a
get_checked  : &Vec a -> Size -> Res Fault a
```

Reach for these **only when the value comes from untrusted external input**. When the bound
derives from an already constrained input, prove it statically instead — choosing the
checked form there is the mistake a generator makes most often.

---

## What the prelude does not have

- **No map, set or dictionary.** Nothing associates a key with a value. This needs a
  runtime type, not a library written in Vibelang.
- **No modulo**, and no other arithmetic beyond the table above.
- **No record pattern.** The grammar lists one; the parser rejects it. Project with `.`.
- **No postcondition syntax.** A fact established inside a function does not leave it,
  except for one inferred case: `refine::postconditions` derives per-constructor payload
  facts of the form `len payload >= k` from a body and assumes them at call sites. That
  vocabulary is one predicate wide.
- **No `import`, alias or visibility.** Qualification at the use site is the whole module
  system.
- **`ghost` declarations** parse and are excluded from codegen — and today also from
  obligation generation, so they teach the solver nothing. Not yet a useful tool.
