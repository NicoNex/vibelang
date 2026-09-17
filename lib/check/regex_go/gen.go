// Generates a random corpus for the differential test: patterns from the
// syntax lib/regex.vibe supports, subjects over a small alphabet that includes
// a newline and a multi-byte code point.
//
//	go run gen.go 5000 1 > random.txt
package main

import (
	"fmt"
	"math/rand"
	"os"
	"regexp"
	"strconv"
	"strings"
)

var atoms = []string{"a", "b", "c", "A", ".", "[ab]", "[^a]", "[a-c]", `\d`, `\w`, `\s`, `\W`, `\b`, `\B`, "^", "$", `\A`, `\z`, "é", "[[:alpha:]]", `\.`, "x", "", `\n`}
var reps = []string{"", "", "", "*", "+", "?", "*?", "+?", "??", "{2}", "{1,2}", "{0,}", "{2,3}?"}
var flags = []string{"", "", "(?i)", "(?m)", "(?s)", "(?U)", "(?im)"}

func expr(r *rand.Rand, depth int) string {
	n := 1 + r.Intn(3)
	var b strings.Builder
	for i := 0; i < n; i++ {
		var a string
		switch k := r.Intn(10); {
		case depth < 3 && k == 0:
			a = "(" + expr(r, depth+1) + ")"
		case depth < 3 && k == 1:
			a = "(?:" + expr(r, depth+1) + "|" + expr(r, depth+1) + ")"
		case depth < 3 && k == 2:
			a = "(?P<g" + strconv.Itoa(r.Intn(3)) + strconv.Itoa(depth) + strconv.Itoa(i) + ">" + expr(r, depth+1) + ")"
		default:
			a = atoms[r.Intn(len(atoms))]
		}
		b.WriteString(a)
		if a != "" && a != "^" && a != "$" {
			b.WriteString(reps[r.Intn(len(reps))])
		}
	}
	if r.Intn(4) == 0 {
		b.WriteString("|" + atoms[r.Intn(len(atoms))])
	}
	return b.String()
}

func main() {
	count, _ := strconv.Atoi(os.Args[1])
	seed, _ := strconv.Atoi(os.Args[2])
	r := rand.New(rand.NewSource(int64(seed)))
	subj := []string{"a", "b", "c", "A", " ", "\n", "é", "1", "x", ".", "_"}
	tpls := []string{"-", "<$0>", "$1", "${g000}", "$$", "[$2]"}
	for i := 0; i < count; i++ {
		p := flags[r.Intn(len(flags))] + expr(r, 0)
		if _, err := regexp.Compile(p); err != nil {
			continue
		}
		var s strings.Builder
		for j := r.Intn(12); j > 0; j-- {
			s.WriteString(subj[r.Intn(len(subj))])
		}
		fmt.Printf("%s\t%s\t%s~\n", p, s.String(), tpls[r.Intn(len(tpls))])
	}
}
