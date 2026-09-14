# Vibelang

English: [README.md](README.md)

**Un linguaggio di programmazione che non è per te.**

Vibelang non è pensato per essere scritto da un umano. È pensato per essere generato da un LLM, verificato da un compilatore spietato, e letto da te — che ormai fai il revisore, non l'autore. Se questo ti sembra un declassamento, hai capito bene il progetto.

```
mod Ledger

parse (ln:&Str) : Res Err Tx =
  ?split ',' ln
   |[s,q,p] -> ?(parse_u32 q, parse_f64 p)
                |(Some n, Some v) -> mk (dup s) n v
                |_                -> Er (Num (dup ln))
   |_       -> Er (Bad (dup ln))
```

Non è C con lo zucchero tolto. È la sintassi con la varianza più bassa possibile per un generatore statistico, unita a un type checker che non lascia passare niente — mai un `null`, mai un overflow silenzioso, mai un `match` a metà.

---

## Perché esiste

I linguaggi che usiamo oggi sono stati ottimizzati per 60 anni per una cosa: renderli piacevoli da scrivere e leggere per un umano. Zucchero sintattico, più modi di dire la stessa cosa, convenzioni impliciti dedotte dal contesto. Tutta roba che un programmatore esperto ama e un modello linguistico paga cara — ogni ambiguità sintattica è un bivio dove il generatore può sbagliare strada, e ogni errore è un altro giro di retry, cioè altri token, altra latenza, altri soldi.

Vibelang parte da una domanda diversa da quella che si è sempre posta la progettazione dei linguaggi: non "quanto è comodo da scrivere", ma **quanti token servono, in totale — generazione più diagnostica più correzioni — prima di arrivare a un programma corretto.** Non la brevità del sorgente. Il costo reale del ciclo genera-compila-correggi.

Da questo obiettivo derivano tutte le scelte:

- **una sola forma valida per ogni programma** — il parser rifiuta le varianti, non le normalizza, perché se esistono due modi di scrivere la stessa cosa il generatore deve scegliere, e scegliere è dove nascono gli errori;
- **niente regole contestuali** — il significato di un token non dipende da dove si trova nel file;
- **fallimento in compilazione, mai a runtime** — un indice fuori range, una divisione per zero, un `match` non esaustivo sono errori di compilazione, non sorprese in produzione alle tre di notte;
- **diagnostica strutturata con un campo `fix` applicabile meccanicamente** — così il generatore corregge in un colpo solo, invece di ritentare alla cieca.

La leggibilità umana non è ignorata: è un obiettivo derivato, ottenuto con strumenti di proiezione (`vibe view --explicit`) invece che scolpita nella sintassi stessa. Tu guardi il rendering. Il generatore scrive la forma canonica.

---

## Cosa garantisce (per progetto)

- **Zero runtime, zero GC.** Compila in C, poi in binario nativo. Nessuna dipendenza oltre alla libc.
- **Puro per default.** Ogni funzione è pura finché non è marcata `E!`. Se una funzione chiama qualcosa di effettoso, deve dichiararlo — nessuna inferenza silenziosa.
- **Ownership affine, senza lifetime.** Un solo proprietario per valore, move di default, prestiti con `&`. Niente annotazioni di lifetime: se un valore deve sopravvivere alla chiamata, o è posseduto o è un errore di compilazione.
- **Totalità.** Ogni funzione deve terminare, provato con una misura decrescente — inferita quando possibile, esplicita (`%espr`) quando no.
- **Refinement types.** Precondizioni e invarianti nella firma stessa (`n:Nat, n<=93`), scaricati su un solver SMT: un obbligo non dimostrato è un errore di compilazione con controesempio concreto, non un warning che ignori.
- **Confine C esplicito e bidirezionale.** `ext c` per importare qualsiasi libreria C, `exp c` per esportare funzioni Vibelang con ABI C. Il confine è l'unico punto dove le garanzie finiscono, ed è sintatticamente visibile.
- **Diagnostica pensata per essere letta da una macchina prima che da te**, con un campo `fix` meccanico quando la riparazione è meccanica.

Nota di onestà: quanto segue è l'obiettivo di design. Più sotto, nella sezione *Stato del progetto*, trovi esattamente cosa di questa lista è già vero oggi e cosa è ancora carta.

---

## Quickstart

Oggi il repository contiene il **frontend** del compilatore bootstrap (lexer, parser, type checker) come libreria Rust. Non esiste ancora il binario `vibe`: niente `cargo install`, niente eseguibile da lanciare su un `.vibe` e ottenere un binario nativo. Quello che segue sotto "UX prevista" è il comando che il progetto punta a offrire — modellato sul toolchain di Go — non quello che gira oggi.

### Cosa puoi fare oggi

```bash
git clone <questo-repo> vibelang
cd vibelang
cargo build          # compila la libreria (lexer, parser, checker)
cargo test           # esegue i test del frontend, se presenti
```

Il crate si chiama `vibec`, `Cargo.toml` dichiara già il binario `vibe` puntato a `src/main.rs`: quel file non esiste ancora, è il prossimo pezzo da scrivere (vedi *Roadmap*).

### UX prevista (non ancora implementata)

```bash
vibe new mioprogetto        # scaffold di un modulo .vibe
vibe run ledger.vibe        # compila (via C) ed esegue, un colpo solo
vibe build                  # emette il binario nativo
vibe check --diag=json      # solo type-check, diagnostica in JSON per un agente
vibe view ledger.vibe --explicit   # mostra tipi inferiti, prove, copie implicite
```

Un binario statico, nessun sistema di build, nessun file di configurazione. Come `go build`, non come `cargo` con il suo grafo di feature flag.

---

## Un programma Vibelang

Questo è il programma di riferimento della specifica (Appendice A), quello che ogni fase dell'implementazione deve far compilare. Legge un CSV di transazioni, valida ogni riga, e stampa totale, media e riga più alta — senza un solo controllo esplicito di lista vuota in `mean` o `top`, perché il compilatore ha già provato che non serve.

```
mod Ledger

ext c "stdio.h"
  puts : &CStr -> E! I32

type Tx  = { sku:Str, qty:U32, price:F64, qty>0, price>0.0 }
type Err = Bad Str | Num Str | Void

parse (ln:&Str) : Res Err Tx =
  ?split ',' ln
   |[s,q,p] -> ?(parse_u32 q, parse_f64 p)
                |(Some n, Some v) -> mk (dup s) n v
                |_                -> Er (Num (dup ln))
   |_       -> Er (Bad (dup ln))

mk (s:Str) (n:U32) (v:F64) : Res Err Tx =
  ?(n>0 && v>0.0)
   |True  -> Ok {sku=s, qty=n, price=v}
   |False -> Er (Bad s)

amt   (t:&Tx)      : F64 = t.price * f64 t.qty
total (ts:&Vec Tx) : F64 = ts |> map amt |> sum

mean (ts:&Vec Tx, len ts>0) : F64 = total ts / f64 (len ts)
top  (ts:&Vec Tx, len ts>0) : &Tx = ts |> max_by amt

load (p:&Str) : E! Res Err (Vec Tx) =
  txt <- read p
  ?txt |> lines |> map parse |> seq
   |Er e  -> Er e
   |Ok [] -> Er Void
   |Ok ts -> Ok ts

main : E! Unit =
  r <- load "ledger.csv"
  ?r |Er e  -> warn (show e)
     |Ok ts -> out (fmt "n={} tot={} avg={} top={}"
                        (len ts) (total &ts) (mean &ts) (top &ts).sku)

exp c mean, total
```

Nota rispetto alla specifica pubblicata: la bozza usa `u32` e `f64` come conversioni di parsing sovraccaricate sulla stringa. Il compilatore bootstrap non fa overloading (è un principio del linguaggio, non solo un limite dell'implementazione attuale), quindi il parsing di stringa è `parse_u32` / `parse_f64` : `&Str -> Opt U32` / `&Str -> Opt F64`. Il codice sopra è già aggiornato a questa forma.

Il punto centrale dell'esempio: `mean` e `top` richiedono `len ts > 0` nella loro firma, ma nel `main` nessuno lo controlla esplicitamente — perché `load` ha già escluso il ramo `Ok []`, e quell'informazione entra nel contesto del solver per il ramo `Ok ts`. Un controllo ridondante non è solo inutile: è un errore che il compilatore non lascerebbe compilare come "necessario", segnalandolo come dead branch.

---

## Interoperabilità C

Il confine C è deliberatamente l'unico punto dove le garanzie del linguaggio finiscono — e deve essere visibile a colpo d'occhio, non nascosto dentro un binding generato.

**Importare una libreria C:**

```
ext c "sqlite3.h" link "sqlite3"
  sqlite3_open : &CStr -> E! I32
  sqlite3_exec : (n:Size, n>0) -> E! I32
```

`link` porta il nome della libreria da passare al linker; `pkg` fa lo stesso con un pacchetto `pkg-config`. Ogni firma `ext c` è obbligatoriamente `E!`: il typechecker non ha modo di sapere cosa fa una funzione C, quindi assume il caso peggiore per costruzione. I refinement su una firma `ext` sono assunti, non provati — verificati sui punti di chiamata Vibelang, non oltre il confine.

**Esportare verso C:**

```
exp c mean, total
```

Genera simboli con ABI C e un header, con due soli qualificatori di ownership possibili nella firma esportata: `own T` (il chiamante C prende la proprietà, viene esportata anche `<nome>_free`) e `ref T` (prestito valido solo per la durata della chiamata). Le precondizioni finiscono nell'header come commento, con un avviso esplicito che oltre il confine non sono verificate:

```c
/* mean — pre: len(ts) > 0   NON VERIFICATA OLTRE IL CONFINE */
double Ledger_mean(const Ledger_Vec_Tx *ts);
```

---

## Diagnostica

Il compilatore emette prosa leggibile per default. Con `--diag=struct` (o `--diag=json`, per un agente) emette lo stesso contenuto in forma di termine, con un percorso semantico stabile invece che un numero di riga, e — dove la riparazione è meccanica — un campo `fix` applicabile senza intervento umano:

```
✗ Ledger.mean.body/2 div0 ⊨ len ts == 0
  at: ledger.vibe:19:34
  msg: division may fail: divisor could be zero
  fix: Ledger.mean.sig += len ts > 0
```

```
✗ Ledger.parse.match nonexhaustive ⊨ missing [_, _]
  at: ledger.vibe:9:3
  msg: match is not exhaustive
  fix: add arm |_ -> Er (Bad (dup ln))
```

Il campo `fix` è, per la metrica di §0 della specifica, il punto di maggior rendimento dell'intero design: chiude il ciclo di correzione in una singola diagnostica invece che in un retry alla cieca. Un fallimento di refinement porta sempre un controesempio concreto dal modello SMT — mai un warning generico da interpretare.

---

## Stato del progetto

**Pre-alpha.** Non c'è ancora un binario `vibe`, non c'è emissione di codice C, non c'è un solo `.vibe` compilato end-to-end. Quello che segue è lo stato reale della libreria in `src/`, non un obiettivo travestito da changelog.

**Implementato:**

- lexer e parser completi per la grammatica della specifica, incluse le regole di canonicità (P1): indentazione a 2 spazi, niente parentesi ridondanti, niente riga vuota multipla — il parser *rifiuta*, non normalizza;
- AST e risoluzione dei nomi;
- type checker con inferenza Hindley–Milner;
- ADT con payload, record con campi, pattern matching con controllo di esaustività;
- propagazione degli effetti (`E!`): una funzione che ne chiama una effettosa deve dichiararsi tale, senza inferenza silenziosa;
- refinement nella firma type-checkati come espressioni booleane nello scope dei parametri (la parte *sintattica* della §7 della specifica);
- parsing di `ext c` con `link`/`pkg` e di `exp c`;
- diagnostica strutturata (`--diag=struct` / `--diag=json`) con path semantico, witness e campo `fix` meccanico dove applicabile.

**Non ancora implementato:**

- il binario `vibe` stesso e la sua CLI (`run`, `build`, `check`, `new`, `view`, `patch`);
- emissione di C e qualsiasi forma di compilazione a binario nativo;
- il borrow checker e il riuso in-place;
- il controllo di terminazione (inferenza della misura, verifica di `%`);
- lo scarico effettivo degli obblighi di refinement su un solver SMT — oggi sono type-checkati come booleani, non dimostrati;
- la libreria runtime minimale (arene, `Vec`, conversioni `Str`/`CStr`);
- qualunque sistema di moduli oltre un singolo file.

Se una sezione sopra descrive un comportamento e non lo vedi in questa lista, è design, non funzionalità.

---

## Roadmap

Le fasi ricalcano quelle della specifica (§15), pensate per far emergere presto i rischi veri invece di rimandarli.

| Fase | Contenuto | Stato |
|---|---|---|
| 0 | Validazione dell'ipotesi: 50 task generati in Rust/Haskell vs. sintassi Vibelang mockata, misurando token fino al programma corretto | non tracciata in questo repo |
| 1 | Frontend — lexer, parser con verifica di canonicità, AST, risoluzione nomi | **fatto** |
| 2 | Typechecker — inferenza HM, ADT, esaustività, effetti | **fatto** (refinement type-checkati, non ancora dimostrati) |
| 3 | Emissione C — senza ownership, alloca e non libera, per validare mappatura dei tipi, FFI e runtime minimo | non iniziata |
| 4 | Ownership — borrow checker, riuso in-place, escape analysis | non iniziata |
| 5 | Totalità — inferenza della misura, controllo di terminazione | non iniziata |
| 6 | Refinement — generazione obblighi, integrazione Z3, controesempi, diagnostica strutturata | parzialmente: la sintassi e il type-check dei refinement esistono, manca il solver |
| 7 | Tooling — viste (`view`), patch strutturate (`patch`), superficie per l'agente | non iniziata |

Le fasi 1–3 sono quelle che rendono il linguaggio già usabile per scrivere e compilare programmi reali; questo repo è a metà della fase 2. Le fasi 4–6 sono indipendenti tra loro e possono procedere in ordine diverso.

---

## Licenza

GPL-3.0-or-later (vedi [LICENSE](LICENSE)). Il progetto resta pre-alpha: la superficie del linguaggio, la sintassi e la lista dei principi normativi (§1 della specifica) possono ancora cambiare — la licenza no.

---

*L'editor di testo è obsoleto. Tu sei il revisore. Il compilatore non fa sconti a nessuno dei due.*
