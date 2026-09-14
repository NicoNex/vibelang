![Vibelang](assets/banner.svg)

# Vibelang

<sub>English: [README.md](README.md)</sub>

**Un linguaggio di programmazione che non è per te.**

Vibelang non è progettato per essere piacevole da scrivere. È progettato per essere *generato*: una sola forma canonica per programma, zero zucchero sintattico, un type checker che rifiuta ciò di cui non sa rendere conto, e una metrica per cui il resto della progettazione dei linguaggi non ha mai ottimizzato — il totale dei token bruciati, generazione più diagnostica più tentativi, prima che un programma sia corretto.

Il pubblico di riferimento è un modello statistico. Non ha opinioni su dove vanno le graffe, non apre issue sull'operatore ternario e non ha bisogno di onboarding. Progettare per lui è, in questo senso stretto, un sollievo.

Adesso il revisore sei tu, non l'autore. Se ti sembra un declassamento, hai capito bene il progetto.

```
mod Ledger

parse (ln:&Str) : Res Err Tx =
  ?split ',' ln
   |[s,q,p] -> ?(parse_u32 q, parse_f64 p)
                |(Some n, Some v) -> mk (dup s) n v
                |_                -> Er (Num (dup ln))
   |_       -> Er (Bad (dup ln))
```

Non è C con lo zucchero tolto. È la sintassi a varianza minima per un generatore statistico, imbullonata a un compilatore che trasforma un file `.vibe` in un eseguibile nativo passando per C. Quel compilatore esiste, e gira:

```console
$ vibe run examples/ledger.vibe
n=3 tot=20.75 avg=6.91667 top=b
```

In questo repository non c'è nessun modello. Niente API key, niente inferenza, niente finestra di chat, niente che completi da solo. Vibelang è la metà senza gloria dell'accordo: quella che legge ciò che ha scritto la macchina e le comunica, in un formato stabile e leggibile da una macchina, che ha sbagliato.

È presto. La sezione [Stato](#stato) dice esattamente quanto, nell'unica forma utile: un elenco di ciò che funziona, uno di ciò che funziona a metà e uno di ciò che è ancora carta.

---

## Se niente di tutto questo ti dice niente

Qui sotto non servono prerequisiti. E non c'è niente di semplificato: è solo spiegato per esteso.

Un **linguaggio di programmazione** è la notazione in cui si scrivono le istruzioni. Un **compilatore** è il programma che trasforma quella notazione in qualcosa che un processore può davvero eseguire e — la metà che conta qui — si rifiuta di farlo quando le istruzioni non tornano. Un compilatore passa gran parte della propria vita lavorativa a dire di no. Quello di Vibelang la passa a dire di no più spesso, di proposito.

**La premessa.** Ogni linguaggio di cui hai sentito parlare è stato progettato attorno a una persona che lo scrive — decenni di ricerca su quale notazione gli umani trovino leggibile, memorizzabile, indulgente. Vibelang dà per scontato che la prima stesura la scriva una macchina, e che la legga una persona che poi la approva o la rimanda indietro. Ogni scelta in questa pagina è quell'unica inversione, portata fino in fondo.

**Un solo modo di scrivere ogni cosa.** Quasi tutti i linguaggi permettono di scrivere la stessa idea in tre o quattro modi, e a scegliere è il gusto. Un generatore non ha gusto; ha una distribuzione di probabilità. Due grafie ugualmente valide sono un lancio di moneta, e un lancio di moneta in mezzo a un programma è un bug che aspetta il suo turno. Perciò il parser rifiuta le varianti invece di accettarle e mettere in ordine dopo: le dichiarazioni iniziano in colonna 1, le parentesi che non servivano sono un errore, una seconda riga vuota consecutiva è un errore. Gli altri linguaggi distribuiscono un formatter, cioè uno strumento che serve a perdonarti. Il formatter di Vibelang è il messaggio di errore.

**Quattro cose che non compilerà.** In parole povere:

- **Una scelta a cui manca un caso.** Il codice gestisce il rosso e il verde; esiste anche il blu. Quasi tutti i linguaggi lasciano che una cosa così vada in produzione, e cascano la prima volta che si presenta il blu. Vibelang rifiuta il file e fa il nome del blu.
- **Usare un valore dopo averlo ceduto.** Ogni valore ha esattamente un proprietario. Lo passi a qualcun altro e non ce l'hai più; usarlo di nuovo è vendere due volte la stessa auto, e il compilatore è lì in piedi alla seconda vendita.
- **Un ciclo che potrebbe non fermarsi mai.** Tutto ciò che chiama sé stesso deve esibire una quantità che cala in senso stretto a ogni chiamata, così che il fondo sia raggiungibile. Niente quantità che cala, niente programma.
- **Dividere per qualcosa che potrebbe essere zero.** Le operazioni controllate restituiscono un risultato che è un numero oppure un fallimento, e davanti al fallimento non c'è modo di guardare da un'altra parte.

La parola portante è **prima**. Nessuno di questi è un controllo a runtime che scatta quando il caso brutto finalmente arriva. Si chiudono mentre non sta girando niente, e valgono per ogni esecuzione che potrà mai avvenire. Lo standard del settore è venire a sapere le stesse cose alle tre di notte, da un cliente, oppure da nessuno dei due — e un test copre solo i casi che a qualcuno è venuto in mente di scrivere.

(La forma più forte della quarta — dimostrare che il divisore non è mai zero, così che non serva alcun controllo — la esegue un solver esterno ed è oggi opt-in. [Stato](#stato) è preciso su quale garanzia è quale, che è l'intero motivo per cui quella sezione esiste.)

**Errori indirizzati a una macchina.** Un errore di compilazione ordinario è una lamentela: ecco cosa non va, buona fortuna. Una diagnostica Vibelang porta un campo `fix`, che non è un consiglio ma la modifica — il testo esatto da aggiungere, e dove va messo. La differenza sta in chi legge. Una persona colma da sola la distanza fra «questo match non è esaustivo» e la riparazione, usando il buon senso, e quasi non se ne accorge. Un generatore paga quella distanza con un tentativo intero: rigenerare, ricompilare, rileggere la lamentela, sperare. Un `fix` chiude invece il ciclo dentro una sola diagnostica. Per lo stesso motivo ogni errore è indirizzato con un percorso semantico (`Color.show_c.match`) invece che con un numero di riga — le righe le ha già spostate l'ultima modifica del generatore stesso.

**Perché compila in C.** Vibelang non emette codice macchina per conto suo. Emette C, e lo consegna al compilatore C che hai già sulla macchina. C è la porta d'ingresso di quasi tutte le librerie esistenti — decoder di immagini, database, crittografia, il sistema operativo sotto a tutte loro — quindi passare per C eredita sessant'anni di lavoro altrui invece di spendere il primo decennio a rifarlo male. Il costo è una dipendenza da un compilatore C. L'alternativa era un linguaggio capace di fare aritmetica e nient'altro.

Nessuna di queste scelte rende il linguaggio più gradevole da battere a tastiera. Non è una svista. È il progetto, e tu stai dall'altra parte.

---

## Perché esiste

I linguaggi che usiamo sono stati ottimizzati per sessant'anni per una cosa sola: essere piacevoli da scrivere e da leggere per un umano. Zucchero sintattico, più modi di dire la stessa cosa, convenzioni dedotte dal contesto. Un programmatore esperto ama tutto questo. Un modello linguistico lo paga tutto — ogni ambiguità sintattica è un bivio dove il generatore può sbagliare strada, e ogni strada sbagliata è un altro retry, cioè altri token, altra latenza, altri soldi.

Tutta quella ricerca è ospitalità, e l'ospite ha smesso di presentarsi a scrivere la prima stesura.

Vibelang si pone quindi una domanda diversa. Non «quanto è comodo da scrivere», ma **quanti token servono, dall'inizio alla fine, per arrivare a un programma corretto.** Non la brevità del sorgente. Il costo del ciclo genera-compila-correggi.

Da lì discende ogni decisione di progetto:

- **esattamente una forma valida per ogni programma** — il parser *rifiuta* le varianti invece di normalizzarle, perché se ci sono due modi di scrivere la stessa cosa il generatore deve scegliere, e scegliere è dove nascono gli errori;
- **niente regole contestuali** — il significato di un token non dipende da dove si trova nel file;
- **fallimento in compilazione, non alle tre di notte** — un `match` non esaustivo, un uso dopo il move, una precondizione non dimostrata sono errori, non sorprese;
- **diagnostica con un campo `fix` applicabile meccanicamente** — così il tentativo successivo è una modifica, non una scommessa.

La guida di stile è di conseguenza vuota, e il formatter è il parser che rifiuta il file. Il bikeshed è un errore di parsing.

La leggibilità umana non è ignorata. È un obiettivo *derivato*, da servire con strumenti di proiezione invece che scolpito nella sintassi. Tu leggi il rendering; il generatore scrive la forma canonica. (Quegli strumenti sono `vibe view`. Vedi [Stato](#stato).)

---

## Cosa impone il compilatore oggi

Questa è la sezione in cui il README di un linguaggio di solito elenca aggettivi. Qui è un elenco di cose che ti fermeranno, tutte verificate dal codice in `src/` e coperte da `cargo test`.

- **Forma canonica, imposta e non normalizzata.** Le dichiarazioni iniziano in colonna 1; le parentesi ridondanti sono un errore; due righe vuote di fila sono un errore. Il parser rifiuta e restituisce un `fix`; non riformatta mai in silenzio. (Oggi tre regole, non l'elenco completo della §3.1 della specifica.)
- **Inferenza Hindley–Milner.** L'algoritmo classico che deduce ogni tipo da come un valore viene usato, invece di farselo dire. Inferenza completa sugli ADT — *tipi di dato algebrici*, cioè un tipo dichiarato come lista fissa di alternative, ciascuna delle quali può portare dati — con payload, record, tuple e liste. Le firme si dichiarano; i corpi si inferiscono.
- **Pattern matching esaustivo.** Un costruttore mancante è un errore di compilazione che nomina il costruttore e il ramo da aggiungere.
- **Effetti, propagati non inferiti.** Una funzione è pura finché non è marcata `E!`. Chiamare qualcosa di effettoso da una funzione pura è un errore; il compilatore non ti promuove in silenzio.
- **Terminazione.** Ogni funzione ricorsiva ha bisogno di una misura che decresce a ogni chiamata. Il compilatore la inferisce quando un parametro decresce sintatticamente, e chiede `%expr` quando non ci riesce.
- **Uso affine dei valori posseduti.** *Affine* vuol dire che un valore può essere usato una volta e non due. I parametri `&` sono prestiti — dati in prestito per la durata della chiamata e ancora tuoi dopo; tutto il resto è posseduto e viene consumato dal primo uso. Usarlo due volte è un errore il cui `fix` è `&x` oppure `dup x`. Un aggiornamento `{r with ...}` su un record posseduto in modo unico muta sul posto invece di copiare.
- **Aritmetica e indicizzazione che possono fallire, dentro il sistema di tipi.** `add_checked`, `sub_checked`, `mul_checked`, `div_checked` e `get_checked` restituiscono `Res Fault a`, quindi overflow, divisione per zero e indice fuori range sono valori su cui devi fare match. (Il `+` nudo resta non controllato; il piano della specifica è che a scaricare questi obblighi sia il solver di prove descritto più sotto.)
- **Diagnostica pensata prima per la macchina.** `--diag=prose` per te, `--diag=struct` e `--diag=json` per qualunque cosa stia generando il codice.
- **Un binario nativo.** La codegen emette C, `cc` lo linka contro un piccolo runtime C, e ottieni un eseguibile. Niente GC, niente VM, niente sul target oltre a libc e libm.
- **Un confine C esplicito.** `ext c` importa dichiarazioni C con il cablaggio `link` / `pkg-config`; `exp c` emette simboli con ABI C e un header.

### Progettato, non ancora imposto

La specifica descrive più di quanto il compilatore oggi dimostri. Questo elenco ce l'hanno tutti i progetti; quasi tutti lo chiamano roadmap, lo scrivono al futuro e lo spostano in fondo alla pagina. Qui sta subito sotto l'elenco delle funzionalità, perché la distanza fra i due è esattamente ciò a cui serve un type checker:

- **Refinement type** — un tipo che porta con sé una condizione da soddisfare, così che `mean (ts:&Vec Tx, len ts>0)` si legga «un vettore di transazioni, e non è vuoto». Vengono parsati e type-checkati come espressioni booleane nello scope dei parametri, e sono **asseriti a runtime** se non chiedi la prova.
- **Gli obblighi di refinement** vengono generati per divisione, indicizzazione, overflow, invarianti di record e precondizioni sui punti di chiamata, e `vibe check --prove` li scarica con `z3` — un solver SMT, cioè un programma che decide se un insieme di fatti aritmetici e logici può valere tutto insieme, e quindi se una condizione segue già da ciò che è noto. Senza `--prove`, `vibe check` si limita a contare quelli rimasti aperti. Un pattern di costruttore porta il proprio payload dentro al solver, quindi è un ramo `Ok [] ->` precedente a scaricare un `len ts > 0` successivo — nessun controllo esplicito, che è il punto del programma di riferimento.
- **Memoria.** L'allocazione è un bump allocator — un puntatore che cammina in avanti e mai indietro — e gran parte di ciò che distribuisce oggi torna indietro da solo ([Memoria](#memoria) spiega come). La metà che la §4.6 chiede e non ottiene è quella gratis: una closure che non può sopravvivere alla chiamata che l'ha creata dovrebbe stare sullo stack, e oggi ogni closure sta sullo heap. Né esiste un borrow checker completo — l'uso affine è verificato, le regole di aliasing oltre a quello no.

---

## Diagnostica

Quasi tutti i compilatori sono scritti come se si stessero scusando con un umano. Vibelang redige un verbale.

Prosa per default:

```console
$ vibe check color.vibe
vibe: error[match.nonexhaustive]: this match does not cover Blue
  --> color.vibe:6:3
   |
   |   ?k |Red   -> dup "red"
   |   ^
   = counterexample: missing Blue
   = fix: add `|Blue -> ...`
```

Lo stesso contenuto in forma di termine, indicizzato da un percorso semantico stabile invece che da un numero di riga:

```console
$ vibe check color.vibe --diag=struct
vibe: ✗ Color.show_c.match match.nonexhaustive ⊨ missing Blue
  at: color.vibe:6:3
  msg: this match does not cover Blue
  fix: add `|Blue -> ...`
```

E in JSON, per qualunque cosa abbia scritto il file:

```console
$ vibe check color.vibe --diag=json
vibe: [{"path":"Color.show_c.match","code":"match.nonexhaustive","msg":"this match does not cover Blue","file":"color.vibe","line":6,"col":3,"witness":"missing Blue","fix":"add `|Blue -> ...`"}]
```

A reggere il progetto sono tre campi:

- `path` — un indirizzo semantico (`Color.show_c.match`) invece di una coordinata, così un generatore non deve riancorarsi a numeri di riga che la sua stessa ultima modifica ha invalidato;
- `witness` — la cosa concreta che è andata storta, mai una categoria;
- `fix` — presente ogni volta che la riparazione è meccanica, così il ciclo si chiude dentro una sola diagnostica invece che in un retry alla cieca.

Gli errori di ownership hanno la stessa forma:

```console
$ vibe check tests/move.vibe --diag=struct
vibe: ✗ Move.twice own.use_after_move ⊨ first moved at line 5, column 28
  at: tests/move.vibe:5:30
  msg: `s` was already moved
  fix: borrow it here with `&s`, or copy it with `dup s`
```

---

## Stato

Stato onesto di `main` a oggi. Spostare una riga da un elenco al successivo è il modo previsto di aggiornare questa sezione. Niente percentuale di completamento, niente barra di avanzamento, niente trimestri.

**Funziona**

- lexer; parser con imposizione della forma canonica; AST; risoluzione dei nomi
- type checker Hindley–Milner; ADT; record; pattern matching esaustivo
- propagazione degli effetti (`E!`)
- diagnostica strutturata (`--diag=prose|struct|json`) con percorso semantico, witness e `fix` meccanico
- generazione di codice C, e un piccolo runtime C
- FFI `ext c` con `link` / `pkg-config`, ed esportazione `exp c` con header generato
- `vibe build --lib`: un archivio statico più quell'header, linkabile da C senza alcuna chiamata di inizializzazione
- la CLI `vibe`: `check`, `build`, `run`, `view` — da un file `.vibe` a un eseguibile nativo passando per C
- la superficie per l'agente della §13.3: `vibe patch` (percorso semantico, protetto da hash, rifiutato se il risultato non compila più), `vibe deps` (chiamanti e chiamati), `vibe proof` (obblighi aperti per percorso)
- controllo di terminazione: misure inferite, e `%expr` quando l'inferenza si arrende
- obblighi di refinement generati per la §7.2 e scaricati con `vibe check --prove` (richiede `z3` nel PATH), con gli obblighi dimostrati messi in cache in un `.vibe-proofs` accanto al file, indicizzati dall'hash della domanda posta, e un budget del solver per singolo obbligo (`--prove-timeout=`, 5 secondi per default) che dichiara di arrendersi invece di fingere una confutazione (§16.5)
- le viste di proiezione: `vibe view` (forma canonica, identica byte per byte su ogni file `.vibe` del repository, commenti inclusi), `--sig-only`, `--explicit`, `--flow`
- escape analysis per le closure — decidere quali valori sopravvivono alla chiamata che li ha costruiti: una closure il cui valore arriva al risultato possiede le proprie catture, una consumata durante la chiamata le legge
- blocchi `arena a in ...`: una regione con un nome in cui allochi e che butti via intera, e che rilascia tutto ciò che ha allocato quando finisce
- rilascio automatico per frame: una funzione che non può passare un puntatore a C rilascia tutto ciò che ha allocato quando ritorna, e il runtime annulla il rilascio quando il risultato è a sua volta allocato sullo heap
- refinement dei valori legati da un pattern di costruttore: un ramo apprende il tag del costruttore, la lunghezza del payload e l'invariante di record del payload
- programmi su più file: `Money.cents` è tutto il sistema di import, risolto caricando `money.vibe` accanto al file che lo nomina, con namespace piatto e un conflitto segnalato invece che nascosto per shadowing (§9)
- il programma di riferimento dell'Appendice A della specifica compila e gira

**Parziale**

- **ownership** — verifica dell'uso affine di ogni nome posseduto: parametri, binder `let` e `<-`, nomi legati da un pattern, e le catture di una closure che sopravvive alla propria chiamata. Aggiornamento sul posto dei record posseduti in modo unico. Quello che manca è la metà allocativa della §4.6: una closure che non sfugge dovrebbe stare sullo stack, e oggi ogni closure è allocata sullo heap.
- **refinement** — scaricati con `vibe check --prove`, asseriti a runtime altrimenti. `--prove` è opt-in e non il default, e richiede `z3` nel PATH.

**Non ancora**

- allocazione sullo stack per una closure che non sfugge (la §4.6 vuole costo zero; la metà di ownership di quella regola è fatta, la metà allocativa no)

Su parecchie di queste cose si lavora in parallelo, quindi questo elenco si muove più in fretta della prosa che lo precede.

Anche a runtime il compilatore è schietto sulla propria maturità, il che è più di quanto riesca alla maggior parte di noi:

```console
$ vibe run refine.vibe
✗ Refine.half.pre refuted ⊨ n > 0
  this obligation is checked at run time in the bootstrap; fase 6 discharges it statically
```

---

## Provalo in 60 secondi

Nessuna dipendenza — il compilatore è un singolo crate Rust con il grafo delle dipendenze vuoto. Servono una toolchain Rust e un compilatore C.

```bash
git clone <this-repo> vibelang
cd vibelang
cargo build
```

Esegui il programma di riferimento. Legge `ledger.csv` dalla directory di lavoro, quindi lancialo dalla radice del repository:

```console
$ ./target/debug/vibe run examples/ledger.vibe
n=3 tot=20.75 avg=6.91667 top=b
```

Ora guarda il ciclo chiudersi. Scrivi un `match` con un buco dentro:

```bash
cat > color.vibe <<'EOF'
mod Color

type Color = Red | Green | Blue

show_c (k:&Color) : Str =
  ?k |Red   -> dup "red"
     |Green -> dup "green"

main : E! Unit =
  out (show_c &Red)
EOF
```

```console
$ ./target/debug/vibe check color.vibe --diag=struct
vibe: ✗ Color.show_c.match match.nonexhaustive ⊨ missing Blue
  at: color.vibe:6:3
  msg: this match does not cover Blue
  fix: add `|Blue -> ...`
```

Il campo `fix` è tutto il prodotto. Non è un consiglio, è una modifica, ed è indirizzata a qualcosa che non ha bisogno di essere convinto.

Applica quel `fix` — aggiungi `|Blue  -> dup "blue"` sotto gli altri rami — e lo stesso file compila e gira:

```console
$ ./target/debug/vibe run color.vibe
red
```

Poi portati via un eseguibile, e tieniti il C se hai voglia di leggerlo:

```console
$ ./target/debug/vibe build examples/ledger.vibe -o ledger --emit-c
$ ./ledger
n=3 tot=20.75 avg=6.91667 top=b
```

Tutta la CLI sono tre comandi. Niente dashboard, niente directory di plugin, niente in cui fare login:

```
vibe check <file.vibe>            type-check only; silent on success
vibe build <file.vibe> [-o out]   emit C, compile, link
vibe run   <file.vibe> [-- args]  build and execute

  --diag=prose|struct|json        diagnostic rendering (default: prose)
  --emit-c                        keep the generated C next to the output
```

`cargo test` esegue la suite end-to-end: ogni esempio deve passare il type-check, il programma di riferimento deve produrre esattamente l'output qui sopra, un errore di tipo deve riportare `type.mismatch`, un doppio move deve riportare `own.use_after_move`, e un `{r with ...}` su un valore posseduto non deve copiare.

---

## Il programma di riferimento

L'Appendice A della specifica, e il programma che ogni fase dell'implementazione deve continuare a far compilare. Legge un CSV di transazioni, valida ogni riga e stampa conteggio, totale, media e la riga più grande — senza un solo controllo esplicito di lista vuota in `mean` o `top`.

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

`mean` e `top` pretendono `len ts > 0` nelle loro firme, e in `main` non lo controlla nessuno. L'intento di progetto è che `load` abbia già escluso `Ok []`, che questo fatto entri nel contesto del solver sul ramo `Ok ts`, e che un controllo ridondante sarebbe a sua volta un errore di ramo morto.

Sia però chiaro perché compila *oggi*: i refinement vengono type-checkati e poi asseriti a runtime, quindi nessuna prova viene eseguita. Questo programma è l'obiettivo che tiene onesta la pipeline — non la prova che la prova esista.

Una deviazione dalla specifica pubblicata: la bozza scrive `u32` e `f64` come conversioni di parsing sovraccaricate sulle stringhe. Vibelang non ha overloading — è un principio del linguaggio, non una lacuna dell'implementazione — quindi il parsing di stringa è `parse_u32` / `parse_f64` : `&Str -> Opt U32` / `&Str -> Opt F64`. Il codice qui sopra è la forma corretta.

---

## Moduli

Un file, un modulo, e il nome è l'import:

```
mod App

gross (euro:F64) : I64 = Money.vat (Money.cents euro) 22
```

`Money.vat` si risolve caricando `money.vibe` dalla stessa directory. Non c'è una riga `import`, non ci sono alias né percorsi di ricerca: così sparisce un costrutto e, cosa che conta di più, un punto in cui un generatore può tirare a indovinare. La §9 della specifica non dà alla v0.1 nessun sistema di visibilità, quindi i moduli caricati vengono appiattiti in un'unica unità e un nome dichiarato due volte è un errore `mod.duplicate` invece che uno shadowing silenzioso. La diagnostica conserva il modulo a cui appartiene il codice, quindi un obbligo sollevato dentro `Money` resta indirizzato come `Money.half.body/0` anche in un'esecuzione con radice `App`.

Oltre un progetto piccolo tutto questo non scala, e la specifica lo dice da sé. È la cosa più piccola che fa funzionare due file.

---

## Memoria

L'allocazione è un bump allocator. La liberazione non è reference counting e non è un garbage collector: sono due domande sul fatto che qualcosa di ciò che un frame ha allocato ne sia uscito.

La prima la pone il runtime, dinamicamente e a costo zero: un frame si rifiuta di rilasciare quando il proprio risultato è una stringa, un oggetto, un vettore o una closure, perché quel valore è esattamente ciò che è sfuggito. La seconda la si pone staticamente, e c'è una cosa sola da chiedere, perché Vibelang non ha variabili globali né mutazione di valori presi in prestito: *questo frame ha passato un puntatore a C?* C se lo può tenere per tutto il tempo che vuole. Quella contaminazione risale ai chiamanti, dato che il rilascio avviene nel frame più esterno.

Tutto il resto viene rilasciato al ritorno. `examples/churn.vibe` costruisce e butta via duemila volte un vettore da cinquemila elementi:

```console
$ /usr/bin/time -l ./churn
10000000
        1556480  maximum resident set size
```

Lo stesso programma con il rilascio disattivato arriva a un picco di 325 MB. Il numero che conta non è il rapporto, è che sia piatto: il ciclo non cresce più.

Il tetto è la granularità della funzione intera. Un ciclo ricorsivo di coda marca una volta e rilascia una volta, quindi le sue iterazioni continuano ad accumulare fino al ritorno; `arena a in ...` è l'override manuale per quel caso, e una marcatura per iterazione è la strada per migliorare.

---

## Il confine C

Il confine C è deliberatamente l'unico punto in cui le garanzie finiscono, e deve essere visibile a colpo d'occhio invece che sepolto dentro un binding generato.

```
ext c "sqlite3.h" link "sqlite3"
  sqlite3_open : &CStr -> E! I32
  sqlite3_exec : (n:Size, n>0) -> E! I32
```

`link` indica al linker il nome di una libreria; `pkg` ne risolve una tramite `pkg-config`. Ogni firma `ext c` è obbligatoriamente `E!`: il checker non può sapere cosa fa una funzione C, quindi assume il caso peggiore per costruzione. I refinement su una firma `ext` sono *assunti* — verificati sui punti di chiamata Vibelang, e non un passo oltre.

Nella direzione opposta:

```
exp c mean, total
```

emette simboli con ABI C e un header. Le precondizioni finiscono nell'header come commento, con l'avviso allegato:

```c
/* mean — pre: len ts > 0   NOT VERIFIED ACROSS THE BOUNDARY */
double Ledger_mean(VbVal x0);
```

Urlarlo in un commento non è verifica. È il massimo che un header possa fare, e fingere il contrario è il modo in cui le garanzie sgusciano fuori da un progetto.

Un modulo senza `main` è una libreria, e `vibe build --lib` lo dice ad alta voce:

```console
$ vibe build --lib examples/mathlib.vibe
libmathlib.a
$ cc -std=c11 -o use use.c -L. -lmathlib -lm
```

L'archivio arriva con accanto il suo header generato. I wrapper esportati inizializzano da soli il runtime, quindi non c'è nessuna `vibe_init()` che il lato C possa dimenticare. È la direzione a cui il progetto tiene di più: C non è una via di fuga imbullonata alla fine, è il modo in cui sessant'anni di librerie esistenti restano raggiungibili senza che nessuno le riscriva.

La firma esportata oggi passa il valore uniforme di runtime `VbVal`. I qualificatori di ownership `own T` / `ref T` della specifica, e la `<name>_free` generata, non sono ancora implementati.

---

## Per approfondire

- [`vibelang-spec.md`](vibelang-spec.md) — la specifica normativa: obiettivi e non-obiettivi (§0), principi normativi (§1), grammatica e regole di canonicità (§3), ownership (§4), effetti (§5), totalità (§6), refinement (§7), interoperabilità C (§10), fasi di implementazione (§15). Attualmente scritta in italiano. Al generatore non dà fastidio.
- [`README.md`](README.md) — questa pagina in inglese.
- `examples/` — il programma di riferimento e un hello world. `tests/` — la suite end-to-end, che è anche la descrizione più affidabile di cosa faccia davvero il compilatore.

---

## Licenza

GPL-3.0-or-later — vedi [LICENSE](LICENSE). La superficie del linguaggio può ancora cambiare; la licenza no.

---

*L'editor di testo è obsoleto. Tu sei il revisore. Il compilatore non fa sconti a nessuno dei due.*
