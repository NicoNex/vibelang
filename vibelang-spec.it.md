# Vibelang — Specifica del linguaggio

<sub>English: [vibelang-spec.md](vibelang-spec.md)</sub>

> Estensione file: `.vibe` — comando: `vibe`
> Versione documento: 0.1 — bozza di design, non normativa
> Stato: parzialmente implementata. Ogni sezione dichiara cosa fa oggi il compilatore e cosa è ancora design.

---

## 0. Obiettivi e non-obiettivi

### Obiettivi, in ordine di priorità

1. **Minimizzare i token totali fino al programma corretto.** Non la lunghezza del sorgente: generazione + diagnostica + retry. Ogni scelta di design va valutata su questa metrica.
2. **Minimizzare la probabilità di errore di un LLM generatore.** Una sola forma per esprimere ogni cosa; sintassi ad alto prior nel training (famiglia ML/Rust); nessuna regola contestuale.
3. **Correttezza statica massima.** Puro, totale, ownership statica, refinement types con scarico SMT.
4. **Zero runtime, zero GC.** Il binario non dipende da nulla oltre alla libc.
5. **Interoperabilità C bidirezionale** con superficie sintattica minima.
6. **Leggibilità umana come obiettivo derivato**, ottenuta via tool di proiezione, non via design della sintassi.

### Non-obiettivi espliciti

- Non è un linguaggio general-purpose per umani. L'ergonomia umana è delegata al renderer.
- Non supporta strutture cicliche o condivisione arbitraria senza arene.
- Non supporta lifetime polimorfici (vedi §4.4 per il costo di questa scelta).
- Non ha macro, overloading, ereditarietà, riflessione.
- Non punta a performance superiori al C; punta a pareggiarlo.

---

## 1. Principi normativi

Questi principi risolvono le ambiguità di design. In caso di conflitto tra una regola specifica e un principio, vince il principio.

**P1 — Unicità della forma.** Per ogni programma esiste esattamente una rappresentazione testuale valida. Il parser rifiuta forme non canoniche. Non esiste un formatter opzionale: la canonicità è parte della grammatica.

**P2 — Nessuna regola contestuale.** Il significato di un token non dipende dalla posizione nel file, dallo stato del parser, o da dichiarazioni precedenti diverse dal binding dei nomi.

**P3 — Annotazione demand-driven.** Nessuna annotazione è obbligatoria se inferibile. Il compilatore la richiede solo quando l'inferenza fallisce, indicando il punto esatto.

**P4 — Prior alto sui simboli.** Si usano solo simboli ASCII già frequenti nel codice esistente, con il significato che vi hanno già. Nessun glifo Unicode, nessuna reinterpretazione di un simbolo noto.

**P5 — Fallimento in compilazione, mai a runtime.** Ogni condizione che in un linguaggio tradizionale sarebbe un panic (indice fuori range, divisione per zero, overflow, match non esaustivo) è un obbligo di prova. Se non si scarica, non compila.

**P6 — Il confine C è l'unico punto dove le garanzie finiscono,** e deve essere sintatticamente visibile.

---

## 2. Lessico

### 2.1 Tabella dei simboli

Ogni simbolo è scelto per essere un token singolo nei tokenizer comuni e per non collidere con il proprio prior dominante.

| Simbolo | Significato | Prior di riferimento |
|---|---|---|
| `:` | ascrizione di tipo | ML, Rust |
| `->` | freccia (tipo funzione, ramo di match) | ML, Rust |
| `=` | definizione | universale |
| `?` | introduce uno scrutinee di match | nuovo, posizione non ambigua |
| `\|` | ramo di match / alternativa in ADT | ML, Haskell |
| `_` | wildcard | universale |
| `\|>` | pipe forward | F#, Elixir |
| `&` | prestito (borrow) | Rust, C |
| `%` | misura di terminazione | aritmetico, mai strutturale in ML |
| `<-` | bind monadico in blocco effettoso | Haskell, Rust |
| `;` | sequenziamento: termina un bind, concatena due espressioni | C, Rust |
| `{ }` | letterale di record e update | universale |
| `[ ]` | letterale di lista e pattern su lista | universale |
| `( )` | raggruppamento, tupla, firma parametro | universale |
| `,` | separatore | universale |
| `.` | accesso a campo; anche accessor di prima classe | universale |
| `--` | **riservato, non usato** | collide con commento Haskell/Lua |
| `#` | **riservato, non usato** | collide con commento Python/shell |

Commenti: `;;` fino a fine riga. Scelto perché non collide con nessun commento di linguaggio mainstream e non è usato come operatore nella famiglia ML.

### 2.2 Keyword

L'insieme completo. Ogni keyword è una parola inglese comune (prior alto, 1-2 token).

```
mod  ext  exp  type  ghost  let  in  end  E!  own  ref  arena  with
True False
```

Nota: `own` e `ref` compaiono solo nelle firme `exp c`. Dentro il linguaggio l'ownership usa `&` e il default.

### 2.3 Identificatori

`[a-z][a-zA-Z0-9_]*` per valori e funzioni, `[A-Z][a-zA-Z0-9_]*` per tipi e costruttori.

**I nomi restano descrittivi.** È una deviazione deliberata dalla compressione: i nomi portano semantica che guida la generazione. `total_amount` costa quanto `t` in token ma produce meno errori.

---

## 3. Grammatica

EBNF. **L'indentazione non è significativa**: nessuna offside rule, nessun
token `INDENT`/`DEDENT`, nessun conteggio di spazi. Un generatore che sbaglia a
contare gli spazi deve comunque produrre un programma che compila, perché un
errore di parsing costa un retry intero e i retry sono la metrica contro cui
questo linguaggio è ottimizzato.

I costrutti multi-ramo si chiudono con `end`, il sequenziamento monadico usa
`;`. Resta un solo `NL` significativo, e non è una regola di conteggio: un a
capo termina una dichiarazione di primo livello. Senza di esso la
giustapposizione inghiotte il nome della dichiarazione successiva, e
`f : U64 = 1` seguito da `g : U64 = 2` si analizza come `1 g` — un misparse
silenzioso, non un errore. Vale solo fuori da ogni parentesi e da ogni `end`,
solo quando i token già formano un'espressione, e solo se il token successivo
potrebbe iniziarne una: un a capo prima di un operatore binario, prima di un
`|` in una dichiarazione di tipo o prima di una misura `%` è una continuazione.

Da quella regola discende un'asimmetria, ed è l'unico punto in cui la posizione
dell'a capo conta ancora: `-` è anche la negazione unaria e il lexer non
distingue le due, quindi conta come token che può iniziare un'espressione.
`a\n- b` termina dunque la dichiarazione, `a -\nb` no.

```ebnf
module      = "mod" ModName NL { decl } ;

decl        = typedecl | fundecl | extblock | expdecl | ghostdecl ;

typedecl    = "type" TypeName "=" typebody ;
typebody    = record | variants ;
record      = "{" field { "," field } [ "," refine ] "}" ;
field       = name ":" type ;
variants    = variant { "|" variant } ;
variant     = CtorName { type } ;

fundecl     = name { param } [ ":" type ] "=" expr [ measure ] ;
param       = name
            | "(" name { name } ":" type [ "," refine ] ")" ;
measure     = "%" expr ;
refine      = expr ;                    (* espressione booleana decidibile *)

ghostdecl   = "ghost" fundecl ;

expr        = app | match | bind | letexpr | lambda | literal | record
            | expr binop expr | expr "|>" expr ;
app         = atom { atom } ;           (* giustapposizione, curried *)
match       = "?" expr arm { arm } "end" ;
arm         = "|" pattern "->" expr ;
bind        = name "<-" expr ";" expr ; (* solo in contesto E! *)
seq         = expr ";" expr ;           (* `a ; b` è `_ <- a ; b` *)
letexpr     = "let" name "=" expr "in" expr ;
lambda      = "\\" name { name } "->" expr ;

pattern     = literal | name | "_" | CtorName { pattern }
            | "[" [ pattern { "," pattern } ] "]"
            | "(" pattern { "," pattern } ")"
            | record ;

type        = TypeName { type }
            | type "->" type
            | "&" type
            | "E!" type
            | "(" type { "," type } ")" ;

extblock    = "ext" "c" StringLit { extsig } "end" ;
extsig      = name ":" type [ "," refine ] NL ;
expdecl     = "exp" "c" name { "," name } ;
```

### 3.1 Regole di canonicità

La canonicità riguarda la **struttura**, non il layout. Il whitespace non
raggiunge l'AST: indentazione, righe vuote e colonna di partenza di una
dichiarazione sono liberi. Costringere il generatore a contarli produceva
errori di parsing su programmi per il resto corretti, cioè esattamente il costo
che P5 esiste per evitare.

Il parser **rifiuta**, non normalizza:

- parentesi ridondanti;
- `let ... in` dove un binding top-level sarebbe equivalente;
- un `match` o un blocco `ext c` non chiuso da `end`;
- un binding `<-` non terminato da `;`.

Motivazione invariata: se il parser normalizzasse la struttura, esisterebbero
più forme valide per lo stesso programma, violando P1 e introducendo punti di
scelta per il generatore. Il layout non è un punto di scelta, perché non
cambia il programma: `vibe view` ne stampa uno, e nessuno è obbligato a
scriverlo a mano.

---

## 4. Ownership

### 4.1 Modello

Tipi affini. Ogni valore ha esattamente un proprietario. Il passaggio è **move di default**.

### 4.2 La regola unica

> **I parametri prefissati da `&` sono prestiti validi per la durata della chiamata. Tutto il resto è posseduto. Il valore di ritorno è sempre posseduto.**

Non esistono annotazioni di lifetime. Se un valore deve sopravvivere alla chiamata, o è posseduto o è errore di compilazione.

```
amt   (t:&Tx)      : F64 = t.price * f64 t.qty      ;; presta
consume (t:Tx)     : F64 = t.price                  ;; consuma, t non più usabile
```

L'uso affine è verificato oggi su ogni nome posseduto: parametri, binder `let` e
`<-`, nomi introdotti da un pattern, e le catture di una closure il cui valore
raggiunge il risultato della funzione — una closure simile possiede ciò che ha
catturato (§4.6), mentre una consumata durante la chiamata, l'argomento di `map`
per esempio, lo legge soltanto. Un secondo uso è errore, con `fix` `&x` o
`dup x`.

### 4.3 Riuso in-place

`{ r with f = v }` su `r` unicamente posseduto compila a mutazione sul posto, zero allocazioni. Su `r` prestato o condiviso, copia — e il compilatore lo segnala come diagnostica informativa (non errore), perché è la principale sorgente di costo nascosto.

### 4.4 Costo accettato di questa scelta

Non sono esprimibili funzioni che restituiscono un riferimento derivato da un parametro (`fn first(v: &Vec<T>) -> &T` in Rust). Al loro posto: restituire un indice, restituire una copia, o usare un'arena.

**Questa limitazione va validata su codice reale prima di consolidare il design.** Se le API C zero-copy risultano sistematicamente inesprimibili, serve un meccanismo minimo di lifetime — preferibilmente inferito e non scritto.

### 4.5 Arene

Valvola di sfogo per strutture che l'ownership lineare non esprime (grafi, condivisione arbitraria).

```
graph_size (n:Size) : Size = arena a in
  len (rev &(range 0 n))   ;; tutto ciò che alloca in `a` vive fino a fine blocco
```

Deallocazione in blocco a fine scope. Nessun conteggio di riferimenti, nessun tracciamento a runtime.

Accanto alle arene esiste una seconda liberazione, automatica e a granularità di
frame. L'allocazione è un bump pointer; un frame che non può consegnare un
puntatore al C libera al ritorno tutto ciò che ha allocato. Se possa farlo è
deciso staticamente, e in un linguaggio senza globali e senza mutazione di valori
prestati la via d'uscita è una sola: chiamare un simbolo `ext c`, che contamina
la funzione e ogni chiamante, perché la liberazione avviene nel frame più
esterno. La domanda complementare — è il risultato stesso a sfuggire? — è
risolta dal runtime a costo zero: `vb_release` annulla del tutto quando il valore
restituito è una stringa, un oggetto, un vettore, una closure, una stringa C o un
puntatore. Il limite è la granularità: una ricorsione in coda marca una volta e
libera una volta, quindi le sue iterazioni si accumulano fino al ritorno, e
`arena` è l'override manuale per quel caso.

### 4.6 Closure

L'escape analysis è statica e inferita:
- closure che non sfugge dallo scope → stack, costo zero;
- closure che sfugge → possiede le catture, allocata dal chiamante.

Nessuna annotazione. Se l'analisi non riesce a decidere, è errore con richiesta di `move` esplicito.

La metà di ownership di questa regola è oggi applicata; la metà di allocazione
no — ogni closure è allocata sullo heap, e l'analisi decide in modo conservativo
invece di richiedere un `move`. Il drop implicito e l'allocazione su stack per
una closure che non sfugge sono pianificati in
[`docs/static-drop-roadmap.md`](docs/static-drop-roadmap.md), che descrive anche
la rimozione del bump allocator su cui poggia la liberazione di §4.5.

---

## 5. Effetti

### 5.1 Regola

Ogni funzione è **pura per default**. Il tipo `E! T` marca un calcolo che produce `T` toccando il mondo esterno.

```
amt  (t:&Tx) : F64        ;; pura
read (p:&Str) : E! Str    ;; effettosa
```

### 5.2 Propagazione

- Una funzione che chiama una funzione `E!` deve essere `E!`. Non c'è inferenza silenziosa: è errore con fix suggerito.
- `<-` è il bind, utilizzabile solo in corpo `E!`.
- **Tutto ciò che proviene da `ext c` è `E!` per costruzione**, senza eccezioni. Il typechecker non può sapere cosa fa una funzione C.

### 5.3 Granularità

La v0.1 ha un solo effetto (`E!`, indistinto). La suddivisione (`IO`, `Alloc`, `Panic`) è rinviata: aumenta la superficie sintattica e il beneficio va misurato.

---

## 6. Totalità

### 6.1 Requisito

**La divergenza è un effetto.** Da questo discendono due regole, e nessun
costrutto nuovo:

1. **Le funzioni pure devono essere totali.** Ogni funzione pura ricorsiva
   richiede una misura decrescente su un ordine ben fondato, inferita (§6.2) o
   scritta con `%`. È ciò su cui poggia tutto il ragionamento statico: un
   solver che ragiona su una funzione che potrebbe non terminare non sta
   dimostrando niente.
2. **Le funzioni effettose (`E!`) sono esentate.** Possono omettere la misura e
   ricorrere all'infinito. Un event loop o un server sono progettati per non
   terminare, e la firma lo dichiara già: la non-terminazione è uno degli
   effetti che `E!` annuncia.

L'alternativa era aggiungere `while` o `loop`, cioè un costrutto in più e un
punto di scelta in più per il generatore — esattamente ciò che l'Obiettivo 1
vieta. La ricorsione in coda che già esiste basta:

```
serve (port:U16) : E! Unit =
  req <- wait_request port ;
  handle_request req ;
  serve port    ;; ricorsione infinita ammessa: la funzione è E!
```

Il confine è netto e leggibile nella firma. `serve` non termina e lo dice;
`amt (t:&Tx) : F64` termina e lo dice.

### 6.2 Inferenza

La misura è **inferita** quando esiste un parametro scalare che decresce sintatticamente in ogni chiamata ricorsiva. Copre la grande maggioranza dei casi.

```
go (k:Nat) (a b:U64) : U64 =
  ?k |0 -> a
     |_ -> go (k-1) b (a+b)        ;; misura inferita: k
  end
```

Quando l'inferenza fallisce, il compilatore la richiede e si scrive con `%`:

```
sum_to (k:Nat) (n:Nat, k<=n) (acc:U64) : U64 =
  ?(k==n) |True  -> acc
          |False -> sum_to (k+1) n (acc+k)
  end
  %(n-k)
```

### 6.3 Solo misure decrescenti

Non esiste una forma "increasing". Ogni misura crescente con limite si converte meccanicamente: `k` crescente verso `n` è `%(n-k)`. Due forme per lo stesso concetto violerebbero P1 senza guadagno di espressività.

### 6.4 Ricorsione mutua

Ammessa. La misura è una tupla in ordine lessicografico sul gruppo di funzioni mutuamente ricorsive.

---

## 7. Refinement types

### 7.1 Sintassi

I refinement stanno nella firma, dopo il tipo, separati da virgola. Non esiste un blocco `where` separato.

```
fib (n:Nat, n<=93) : U64
mean (ts:&Vec Tx, len ts>0) : F64
```

Sui tipi record sono invarianti, verificati a ogni costruzione e update:

```
type Tx = { sku:Str, qty:U32, price:F64, qty>0, price>0.0 }
```

**Non esiste sintassi per una postcondizione**: un refinement vincola i
parametri, mai il risultato. Un fatto stabilito dentro una funzione non esce
dunque da essa, ed è la lacuna aperta più grande di questa sezione — vedi §16.8
e gli obblighi che restano aperti nel programma di riferimento dell'Appendice A.

### 7.2 Obblighi generati automaticamente (P5)

Il compilatore genera un obbligo di prova, senza che nessuno lo scriva, per:

| costrutto | obbligo |
|---|---|
| `xs[i]` / `get xs i` | `i < len xs` |
| `a / b` | `b != 0` |
| `a + b` su interi macchina | `a + b < MAX` |
| `a - b` su `Nat`/unsigned | `a >= b` |
| costruzione di record con invariante | l'invariante |
| chiamata a funzione con refinement | la precondizione al call site |

### 7.3 Scarico

Tutti gli obblighi vanno a un solver SMT (riferimento: Z3). Un obbligo non scaricato è **errore di compilazione** con controesempio concreto, mai un warning.

È implementato e opt-in: `vibe check --prove` traduce gli obblighi in SMT-LIB 2
e li scarica con il binario `z3` su `PATH`. Senza `--prove`, `vibe check` si
limita a riportare quanti ne restano aperti, e il refinement è asserito a
runtime. I certificati sono messi in cache per hash del testo SMT in un
`.vibe-proofs` accanto al sorgente, e ogni obbligo ha un budget di solver
(`--prove-timeout=`, 5 secondi per default) oltre il quale il compilatore
dichiara di aver rinunciato invece di riportare una refutazione (§16.5).

Due regole governano il contesto in cui un obbligo viene provato:

- un binder che fa shadowing di un nome raffinato riceve un simbolo proprio,
  perché ereditare i fatti del nome esterno proverebbe qualcosa che il programma
  non dice;
- ciò che un pattern di costruttore insegna entra nel contesto del suo ramo — il
  tag, la lunghezza del payload e l'invariante di record del payload. È il
  meccanismo di §8.3, ed è ciò che scarica `len ts > 0` su un ramo `Ok ts` sotto
  un ramo `Ok []`.

### 7.4 Funzioni ghost

Per gli invarianti che il solver non deduce localmente. Non vengono compilate, esistono solo per le prove.

```
ghost fib_spec (n:Nat) : Nat =
  ?n |0 -> 0
     |1 -> 1
     |_ -> fib_spec (n-1) + fib_spec (n-2)
  end
```

### 7.5 Via d'uscita

Dove la prova costa più del beneficio, le operazioni checked spostano il problema a runtime e azzerano l'obbligo:

```
add_checked : U64 -> U64 -> Res Overflow U64
get_checked : &Vec a -> Size -> Res OutOfBounds &a
```

**Nota per il prompt di sistema dell'agente.** La scelta tra provare staticamente e usare la versione checked è la decisione che un generatore sbaglia più facilmente. Regola da esplicitare: provare staticamente quando il bound deriva da un input già vincolato; usare checked quando il valore proviene da input esterno non fidato.

---

## 8. Pattern matching

### 8.1 Forma

```
?scrutinee
 |pat1 -> expr1
 |pat2 -> expr2
end
```

### 8.2 Esaustività

Obbligatoria. Un match non esaustivo è errore di compilazione con l'elenco dei casi mancanti.

### 8.3 Interazione con i refinement (meccanismo centrale)

L'informazione guadagnata da un ramo entra nel contesto del solver per quel ramo. È il meccanismo che elimina i controlli ridondanti:

```
?load path
 |Er e  -> warn (show e)
 |Ok [] -> Er Void
 |Ok ts -> mean &ts        ;; len ts > 0 già provato: i rami Er e Ok [] sono esclusi
end
```

Nessun controllo esplicito di lista vuota, e `mean` è comunque sicura.

È implementato per i pattern di costruttore, letterali, booleani e di lista: un
ramo apprende il tag dello scrutinee, la lunghezza del payload di un pattern di
lista e l'invariante di record di ciò che il payload lega. Un ramo più in basso
apprende inoltre che i rami sopra di lui non sono scattati. Ciò che non attraversa
un confine di funzione è un fatto stabilito in una *funzione chiamata*: servirebbe
una postcondizione, e §7.1 non ne ha la sintassi.

---

## 9. Moduli

Un file, un modulo. `mod Name` in prima riga. Nessun sistema di visibilità in v0.1: tutto ciò che è dichiarato è visibile ai moduli importatori, tranne `ghost`.

L'import è implicito tramite qualificazione: `Ledger.total`. Nessuna keyword `import`, nessun alias — elimina un costrutto e un punto di scelta.

È implementato come specificato. `Ledger.total` carica il modulo `Ledger` da un
file che sta accanto a quello che lo nomina; il nome del file deve corrispondere
al nome del modulo a meno di maiuscole e underscore, quindi `TotalOk` può stare
in `total_ok.vibe` così come in `TotalOk.vibe`, e nient'altro può differire — la
mappatura da nome qualificato a file resta meccanica. I moduli caricati sono poi
appiattiti in un'unica unità, e un nome dichiarato due volte è errore
`mod.duplicate`, non uno shadowing silenzioso.

*(Aperto: questo non scala oltre progetti piccoli. Va rivisto prima della v1.)*

---

## 10. Interoperabilità C

### 10.1 C → Vibelang

```
ext c "stdio.h"
  puts   : &CStr -> E! I32
  malloc : Size -> E! Ptr Byte
end
```

Regole:
- `E!` obbligatorio su ogni dichiarazione (§5.2);
- i refinement su una `ext` sono **assunti, non provati**: verificati sui call site Vibelang, assunti oltre il confine;
- una firma `ext` è un tipo, non una lista di parametri: il refinement si scrive
  dopo il tipo, `name : type, refine` (§3), e può nominare solo ciò che il call
  site stesso nomina — non esiste un binder per l'argomento. La forma a parametri
  `(n:Size, n>0)` di una dichiarazione di funzione non è ammessa qui;
- il compilatore emette l'`#include` corrispondente nel C generato.

### 10.2 Vibelang → C

```
exp c mean, total
```

Genera simboli con ABI C e l'header. Ownership al confine con due soli qualificatori nella firma esportata:

| qualificatore | significato |
|---|---|
| `own T` | il chiamante C prende la proprietà; viene esportata anche `<name>_free` |
| `ref T` | prestito valido solo per la durata della chiamata |

Le precondizioni finiscono nell'header come commento, con avvertenza esplicita che non sono verificate:

```c
/* Ledger.h */
/* mean — pre: len(ts) > 0   NON VERIFICATA OLTRE IL CONFINE */
double Ledger_mean(const Ledger_Vec_Tx *ts);
```

### 10.3 Mappatura dei tipi al confine

| Vibelang | C |
|---|---|
| `U8 U16 U32 U64` | `uint8_t` … `uint64_t` |
| `I8 … I64` | `int8_t` … `int64_t` |
| `F32 F64` | `float`, `double` |
| `Bool` | `bool` |
| `Str` | `struct { const char *p; size_t n; }` |
| `CStr` | `const char *` (NUL-terminated) |
| `&T` | `const T*` |
| `own T` | `T*` |
| record | `struct` con stesso ordine di campi |
| ADT | `struct { uint8_t tag; union {...} v; }` |
| `Res E T` | `struct { bool ok; union { E e; T t; }; }` |

`Str` e `CStr` sono tipi distinti: la conversione è esplicita in entrambe le direzioni e alloca.

---

## 11. Backend

### 11.1 Strategia: emissione C

Non un frontend GCC. Motivazione:

- GCC non ha una plugin API per i frontend; un frontend deve stare in-tree e il processo è in larga parte non documentato. Implica distribuire un GCC patchato.
- Senza GC e senza unwinding, l'IR di Vibelang è quasi isomorfo al C: il gap semantico che un frontend nativo colmerebbe è minimo.
- Emettere C dà GCC, clang, MSVC e ogni toolchain embedded senza lavoro aggiuntivo, e il debug funziona via `#line` senza generare DWARF.

L'emissione C è ciò che viene distribuito. Un secondo backend, nativo, è
pianificato in [`docs/backend-roadmap.md`](docs/backend-roadmap.md): Cranelift
accanto al backend C, così che un programma Vibelang puro non richieda una
toolchain C sulla macchina che lo compila. La regola sotto cui quel piano è
scritto è che **il backend C non è deprecato da esso** — gli header `exp c`,
`--emit-c` e ogni toolchain embedded sono il motivo per cui il C resta un target
pienamente supportato.

Opzione futura: libgccjit (che nonostante il nome fa anche AOT via `compile_to_file`), o LLVM. Da valutare solo se emergono ottimizzazioni non esprimibili in C.

### 11.2 Requisiti sul C generato

- `#line` verso il sorgente `.vibe` su ogni statement;
- self-tail-recursion → loop, emesso direttamente dal compilatore, senza dipendere dall'ottimizzatore;
- tail call mutue → `[[gnu::musttail]]` (GCC/clang: errore se non realizzabile, quindi il fallimento è visibile in compilazione);
- nessun `setjmp`/`longjmp`, nessuna tabella di unwind;
- nessuna dipendenza oltre libc.

### 11.3 Runtime

Libreria statica minimale. Contiene: allocatore per arene, rappresentazione closure, conversioni `Str`/`CStr`, `Vec`. **Nessun GC, nessun refcount, nessun handler di segnale.**

---

## 12. Diagnostica

### 12.1 Due formati, stesso contenuto

Il compilatore emette prosa per default e termini strutturati con `--diag=struct`.

### 12.2 Formato strutturato

```
✗ <path> <codice> ⊨ <controesempio>
  fix: <patch suggerita>
```

Esempi:

```
✗ Ledger.mean.body/2 div0 ⊨ len ts == 0
  fix: Ledger.mean.sig += len ts > 0

✗ Fib.go.arm[1] overflow:U64 ⊨ a=2^63, b=2^63
  need: invariant on (a, b)

✗ Ledger.parse.match nonexhaustive ⊨ missing [_, _]
  fix: add arm |_ -> Er (Bad (dup ln))
```

### 12.3 Requisiti

- Ogni diagnostica porta un **path semantico**, non un numero di riga (stabile sotto inserimento).
- Ogni fallimento di refinement porta un **controesempio concreto** dal modello SMT.
- Dove la riparazione è meccanica (fallimenti di ownership, precondizioni mancanti, rami mancanti), il campo `fix` contiene la patch applicabile.

Il campo `fix` è il punto di maggior rendimento dell'intero design rispetto alla metrica di §0: chiude il ciclo di correzione in una singola diagnostica invece che in un retry.

---

## 13. Tooling

### 13.1 Viste

```
vibe view <path>              ;; forma canonica (identica al file)
vibe view <path> --explicit   ;; tipi inferiti, borrow, prove scaricate, copie implicite
vibe view <path> --sig-only   ;; solo firme
vibe view <path> --flow       ;; pipeline espanse in binding nominati
```

`--explicit` è la risposta al requisito "un umano deve poter vedere cosa c'è nel codice": mostra tutto ciò che il linguaggio omette.

```
go : (k:Nat) -> (a:U64) -> (b:U64) -> U64
go k a b =
  ?k                             ;; exhaustive {0, _}
   |0 -> a
   |_ -> go (k-1) b (a+b)        ;; |- k-1 : Nat   (k != 0)
                                 ;; |- a+b < 2^64  [inv, da n<=93]
  end
  %k                             ;; |- k-1 < k     [inferita]
```

`--sig-only` è la vista economica per il contesto dell'agente: firme di tutto il modulo, corpi solo di ciò che si sta modificando.

Tutte e quattro le viste esistono. La proiezione canonica è byte-identica su ogni
file `.vibe` del repository, commenti inclusi — i commenti sono rimessi a partire
dal sorgente, quindi solo la vista canonica può portarli: le altre tre riscrivono
il programma e li perdono. `--explicit` è più stretta dell'esempio qui sopra:
stampa la firma inferita di ogni dichiarazione e, per una dichiarazione
raffinata, se il refinement è scaricato o verificato a runtime. Le annotazioni di
esaustività e i termini di prova per ramo non sono emessi.

### 13.2 Edit strutturato

```
vibe patch <path> <hash> <nuovo-nodo>
```

L'indirizzamento è per **percorso semantico** (`Ledger.mean.body`), non per indice — stabile sotto inserimento e riordino. L'hash del sottoalbero atteso funge da concorrenza ottimistica: se non combacia, la patch è rifiutata anziché applicata al posto sbagliato.

I file restano di testo e sono la sorgente di verità. L'AST è una cache derivata. La forma canonica (P1) garantisce che una patch strutturata produca un diff testuale minimale, quindi git, grep e code review continuano a funzionare.

È implementato. `vibe patch <file> <path>` senza hash stampa il nodo e il suo
hash; con hash e nuovo nodo lo sostituisce solo se l'hash combacia ancora. Prima
di scrivere qualsiasi cosa il file risultante è rilessato, riparsato e
ricontrollato, e la patch è rifiutata con le diagnostiche — lasciando intatto
l'originale — se il risultato non è un programma.

### 13.3 Superficie da esporre all'agente

| operazione | scopo |
|---|---|
| `view --sig-only` | contesto economico |
| `view --explicit` | ispezione |
| `patch` | edit strutturato |
| `check --diag=struct` | diagnostica in forma di termine |
| `deps` | chiamanti e chiamate |
| `proof` | obblighi SMT aperti su un nodo |

Ogni riga esiste. `vibe deps` riporta chiamanti e chiamate, `vibe proof` elenca
gli obblighi aperti uno per riga sotto lo stesso percorso semantico delle
diagnostiche, e con `--prove` quelli che z3 chiude sono marcati come chiusi.

---

## 14. Libreria standard minima (v0.1)

Solo ciò che serve a scrivere il compilatore stesso e programmi di test.

```
Prelude   Nat U8..U64 I8..I64 F32 F64 Bool Unit Str CStr Ptr Size
          Res e t = Ok t | Er e
          Opt t   = Some t | None
Vec       new push get set len map filter fold each sum max_by min_by
          sort_by rev concat_vec seq range take drop
Str       split lines dup len concat fmt trim starts_with contains slice
          index_of replace lower chr to_cstr from_cstr
Math      abs min max
IO        read read_stdin write out warn argv exit    ;; tutte E!
Checked   add_checked sub_checked mul_checked get_checked div_checked
```

Ogni funzione della stdlib porta i propri refinement:
```
get  (v:&Vec a) (i:Size, i < len v) : &a
div  (a:F64) (b:F64, b != 0.0) : F64
```
Una funzione parziale di cui il bootstrap non sa ancora esprimere il refinement
restituisce invece `Opt`: è il caso di `slice` e `index_of`.

---

## 15. Roadmap di implementazione

Ordine pensato per far emergere presto i rischi veri.

**Fase 0 — Validazione dell'ipotesi (prima di scrivere il compilatore).**
Prendere 50 task rappresentativi, generarli con un LLM in Rust/Haskell e nella sintassi Vibelang mockata (senza compilatore, con correzione manuale). Misurare **token fino al programma corretto**, non lunghezza del sorgente. Se il delta è sotto il 20%, il design va ripensato prima che ci sia un'implementazione a renderlo costoso.

**Fase 1 — Frontend.** Lexer, parser con verifica di canonicità, AST, risoluzione nomi. Nessuna semantica.

**Fase 2 — Typechecker.** Inferenza Hindley-Milner, ADT, esaustività, effetti. A questo punto il linguaggio è già utile e verificabile.

**Fase 3 — Emissione C.** Senza ownership: alloca e non liberare. Serve a validare la mappatura dei tipi, l'FFI e il runtime minimo con programmi reali.

**Fase 4 — Ownership.** Borrow checker, riuso in-place, escape analysis. Qui il C generato inizia a liberare memoria.

**Fase 5 — Totalità.** Inferenza della misura, controllo di terminazione.

**Fase 6 — Refinement.** Generazione obblighi, integrazione Z3, controesempi, diagnostica strutturata con campo `fix`.

**Fase 7 — Tooling.** Viste, patch strutturate, esposizione all'agente.

Le fasi 1–3 producono un linguaggio già usabile. Le fasi 4–6 sono indipendenti tra loro e ordinabili diversamente.

---

## 16. Questioni aperte

Punti dove il design non è risolto e la scelta va presa con dati, non a priori.

1. **Assenza di lifetime (§4.4).** Costo reale sconosciuto finché non si scrive codice che interopera con API C zero-copy. Potrebbe richiedere un meccanismo minimo, preferibilmente inferito.

2. **Invarianti di overflow.** Su aritmetica al limite della rappresentazione il solver chiede spesso una ghost function, che costa più token dell'intera funzione. Da valutare: analisi di range integrata per i casi comuni, così da non delegare tutto all'SMT.

3. **Sistema di moduli.** La qualificazione implicita (§9) non scala. Serve una soluzione che non reintroduca `import`/alias/visibilità — cioè quattro costrutti nuovi.

4. **Granularità degli effetti (§5.3).** Un solo `E!` è probabilmente troppo grossolano per codice reale; suddividerlo aumenta la superficie. Da decidere su codice vero.

5. **Tempi del solver.** In parte risolta. I certificati sono messi in cache per
   hash del testo SMT in un `.vibe-proofs` accanto al sorgente, quindi una
   compilazione i cui obblighi sono tutti in cache non richiede alcun solver, e
   ogni obbligo ha un budget (`--prove-timeout=`, 5 secondi per default) oltre il
   quale il compilatore riporta `refine.budget` — dichiara di aver rinunciato,
   invece di presentare l'assenza di prova come una refutazione. Resta aperta la
   granularità: la cache è indicizzata sul testo della domanda, non sul
   sottoalbero, quindi una modifica non correlata al contesto la invalida.

6. **Strutture cicliche.** Le arene coprono molti casi ma non tutti. Non è chiaro se serva un meccanismo aggiuntivo o se il vincolo sia accettabile.

7. **Concorrenza.** Completamente fuori dallo scope della v0.1. L'immutabilità e l'ownership lineare sono una buona base, ma il design non è stato considerato.

8. **Postcondizioni.** §7.1 ha la sintassi per una precondizione e nessuna per
   una postcondizione, quindi un fatto stabilito dentro una funzione non esce da
   essa. È ciò che lascia aperte le chiamate a `mean` e `top` nel `main`
   dell'Appendice A: `load` ha già escluso `Ok []`, e il chiamante non può
   vederlo. Qualunque sintassi per esprimerlo è un costrutto in più e un punto di
   scelta in più per il generatore, ed è per questo che è una questione e non una
   feature.

---

## Appendice A — Programma di riferimento

Il file che ogni fase dell'implementazione deve far compilare correttamente.

```
mod Ledger

ext c "stdio.h"
  puts : &CStr -> E! I32
end

type Tx  = { sku:Str, qty:U32, price:F64, qty>0, price>0.0 }
type Err = Bad Str | Num Str | Void

parse (ln:&Str) : Res Err Tx =
  ?split ',' ln
   |[s,q,p] -> ?(parse_u32 q, parse_f64 p)
                |(Some n, Some v) -> mk (dup s) n v
                |_                -> Er (Num (dup ln))
               end
   |_       -> Er (Bad (dup ln))
  end

mk (s:Str) (n:U32) (v:F64) : Res Err Tx =
  ?(n>0 && v>0.0)
   |True  -> Ok {sku=s, qty=n, price=v}
   |False -> Er (Bad s)
  end

amt   (t:&Tx)      : F64 = t.price * f64 t.qty
total (ts:&Vec Tx) : F64 = ts |> map amt |> sum

mean (ts:&Vec Tx, len ts>0) : F64 = total ts / f64 (len ts)
top  (ts:&Vec Tx, len ts>0) : &Tx = ts |> max_by amt

load (p:&Str) : E! Res Err (Vec Tx) =
  txt <- read p ;
  ?txt |> lines |> map parse |> seq
   |Er e  -> Er e
   |Ok [] -> Er Void
   |Ok ts -> Ok ts
  end

main : E! Unit =
  r <- load "ledger.csv" ;
  ?r |Er e  -> warn (show e)
     |Ok ts -> out (fmt "n={} tot={} avg={} top={}"
                        (len ts) (total &ts) (mean &ts) (top &ts).sku)
  end

exp c mean, total
```

Proprietà che questo programma esercita: FFI in entrambe le direzioni, ADT con payload, record con invariante, pattern matching annidato ed esaustivo, propagazione degli effetti, prestito multiplo dello stesso valore, copia esplicita da fetta prestata, e — il punto centrale — `mean` e `top` che richiedono `len ts > 0` senza alcun controllo esplicito, perché il ramo `Ok []` è già stato consumato in `load`.

Compila e gira. Sotto `--prove` la maggior parte dei suoi obblighi è scaricata;
i due nel `main` no, per il motivo che dà §16.8.
