// The oracle for lib/regex.vibe. Reads a corpus of `pattern<TAB>subject<TAB>template`
// records, each ended by `~` so that a subject may hold a newline, and prints
// what Go's regexp does with each — find_all, captures, replace, split — in the
// format diff.vibe prints for the Vibelang module. Run by hand, not by
// `cargo test`, which should not need Go:
//
//	go run gen.go 5000 1 > random.txt        # or corpus.txt, syntax.txt
//	go run main.go random.txt > go.out
//	vibe run diff.vibe -- random.txt > vibe.out
//	diff go.out vibe.out
//
// The expected differences are the patterns Go accepts and the module refuses
// on purpose: `\p{...}` and `\pL` (syntax.txt has three).
package main

import (
	"fmt"
	"os"
	"regexp"
	"strings"
)

func main() {
	data, err := os.ReadFile(os.Args[1])
	if err != nil {
		panic(err)
	}
	for _, rec := range strings.Split(string(data), "~") {
		parts := strings.Split(strings.TrimPrefix(rec, "\n"), "\t")
		if len(parts) != 3 {
			continue
		}
		fmt.Println("@@")
		pat, s, tpl := parts[0], parts[1], parts[2]
		re, err := regexp.Compile(pat)
		if err != nil {
			fmt.Println("ERR")
			continue
		}
		var b strings.Builder
		for _, m := range re.FindAllStringIndex(s, -1) {
			fmt.Fprintf(&b, "%d-%d ", m[0], m[1])
		}
		b.WriteString("|")
		if m := re.FindStringSubmatchIndex(s); m != nil {
			for i := 0; i < len(m); i += 2 {
				if m[i] < 0 {
					b.WriteString("-")
				} else {
					b.WriteString("[" + s[m[i]:m[i+1]] + "]")
				}
			}
		} else {
			b.WriteString("nil")
		}
		b.WriteString("|" + re.ReplaceAllString(s, tpl) + "|")
		for _, p := range re.Split(s, -1) {
			b.WriteString("[" + p + "]")
		}
		fmt.Println(b.String())
	}
}
