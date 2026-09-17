#include "vibert.h"

#include <ctype.h>
#include <errno.h>
#include <math.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* ------------------------------------------------------------ allocator
 * calloc and free. Every value is freed at the point its owner dies, which the
 * compiler computes and emits as `vb_dispose` (spec 4.2,
 * docs/static-drop-roadmap.md); there is no region, no mark, nothing rewound in
 * bulk, and nothing that survives to exit because the program stopped.
 *
 * ponytail: malloc is the whole allocator. Size classes and a free list are
 * what a measurement asks for, and the measurement so far — examples/loop.vibe
 * flat at 1.4 MB over two million iterations — has not asked. */

void vb_init(void) {}

static void oom(void) { fputs("vibe: out of memory\n", stderr); exit(70); }

void *vb_alloc(size_t n) {
  void *p = calloc(1, n ? n : 1);
  if (!p) oom();
  return p;
}

static void *grow(void *p, size_t n) {
  p = realloc(p, n);
  if (!p) oom();
  return p;
}

void vb_free(void *p) { free(p); }

/* The byte size of `n` values, refused rather than wrapped: `range 0 (shl 1 62)`
   must run out of memory, not allocate one byte and write past it. */
static size_t vals_size(size_t n) {
  if (n > SIZE_MAX / sizeof(VbVal)) oom();
  return sizeof(VbVal) * n;
}

static VbVal box(VbTag tag, void *p) { VbVal v; v.tag = (uint8_t)tag; v.v.p = p; return v; }

/* ---------------------------------------------------------- constructors */

VbVal vb_unit(void) { VbVal v; v.tag = VB_UNIT; v.v.i = 0; return v; }
VbVal vb_int(int64_t x) { VbVal v; v.tag = VB_INT; v.v.i = x; return v; }
VbVal vb_uint(uint64_t x) { VbVal v; v.tag = VB_UINT; v.v.u = x; return v; }
VbVal vb_float(double x) { VbVal v; v.tag = VB_FLOAT; v.v.f = x; return v; }
VbVal vb_bool(bool x) { VbVal v; v.tag = VB_BOOL; v.v.b = x; return v; }
VbVal vb_char(uint32_t x) { VbVal v; v.tag = VB_CHAR; v.v.c = x; return v; }
VbVal vb_ptr(void *p) { return box(VB_PTR, p); }
VbVal vb_cstr_val(const char *s) { return box(VB_CSTR, (void *)s); }

/* Takes `buf`, which has room for `n + 1` bytes, as the string's own. */
static VbVal str_take(char *buf, size_t n) {
  VbStr *o = vb_alloc(sizeof(VbStr));
  o->n = n;
  o->p = buf;
  buf[n] = 0;
  return box(VB_STR, o);
}
VbVal vb_str(const char *s, size_t n) {
  char *buf = vb_alloc(n + 1);
  if (n) memcpy(buf, s, n);
  return str_take(buf, n);
}
VbVal vb_strz(const char *s) { return vb_str(s ? s : "", s ? strlen(s) : 0); }

static VbObj *obj_alloc(const VbInfo *info, uint32_t tag, uint32_t n) {
  VbObj *o = vb_alloc(sizeof(VbObj));
  o->info = info; o->tag = tag; o->n = n;
  o->f = n ? vb_alloc(vals_size(n)) : NULL;
  return o;
}
VbVal vb_obj(const VbInfo *info, uint32_t tag, uint32_t n, ...) {
  VbObj *o = obj_alloc(info, tag, n);
  va_list ap; va_start(ap, n);
  for (uint32_t i = 0; i < n; i++) o->f[i] = va_arg(ap, VbVal);
  va_end(ap);
  return box(VB_OBJ, o);
}

VbVal vb_clos(VbFn fn, const char *name, uint32_t arity) { return vb_fn(fn, name, arity, 0); }
VbVal vb_fn(VbFn fn, const char *name, uint32_t arity, uint64_t owns) {
  VbClos *c = vb_alloc(sizeof(VbClos));
  c->fn = fn; c->name = name; c->arity = arity; c->nargs = 0; c->owns = owns;
  c->args = arity ? vb_alloc(vals_size(arity)) : NULL;
  return box(VB_CLOS, c);
}

/* Free one value, at the point its owner dies (spec 4.2,
   docs/static-drop-roadmap.md).

   Deep for a vector and for an object, because nothing else points at what
   they hold. Three things had to be true first, and now are: the operations
   that take `&Vec` copy each element rather than its pointer (see the note
   before `vb_push`); a function may no longer return a piece of a borrowed parameter
   (`own.borrow_escapes`); and a value that may still alias one the caller owns
   is not dropped at all, which `vibe view --drops` counts.

   ponytail: a closure stays shallow. Its captures are owned by the frame that
   built it unless the closure escapes, and `own.rs::escapes` knows which — the
   runtime does not. Freeing them here would free the frame's values under it.
   Upgrade path: the escape bit reaches the runtime, or codegen emits the deep
   free for the closures it knows own their captures.

   `VB_CSTR` and `VB_PTR` are C's and are not freed here at all, which is the
   same line `vb_dup` draws. */
void vb_dispose(VbVal v) {
  switch (v.tag) {
    case VB_STR: { VbStr *s = v.v.p; vb_free(s->p); vb_free(s); break; }
    case VB_VEC: {
      VbVec *w = v.v.p;
      for (size_t i = 0; i < w->n; i++) vb_dispose(w->a[i]);
      vb_free(w->a);
      vb_free(w);
      break;
    }
    case VB_OBJ: {
      VbObj *o = v.v.p;
      for (uint32_t i = 0; i < o->n; i++) vb_dispose(o->f[i]);
      vb_free(o->f);
      vb_free(o);
      break;
    }
    case VB_CLOS: { VbClos *c = v.v.p; vb_free(c->args); vb_free(c); break; }
    default: break;
  }
}

/* -------------------------------------------------------------- failures */

void vb_fail(const char *path, const char *what) {
  fprintf(stderr, "✗ %s %s\n", path ? path : "<program>", what);
  exit(70);
}
void vb_require(bool cond, const char *path, const char *what) {
  if (!cond) {
    fprintf(stderr, "✗ %s refuted ⊨ %s\n", path ? path : "<program>", what);
    fputs("  this obligation is checked at run time in the bootstrap; phase 6 discharges it statically\n",
          stderr);
    exit(70);
  }
}

/* -------------------------------------------------------------- unboxing */

static void expect(VbVal v, VbTag t, const char *what) {
  if (v.tag != t) vb_fail("<runtime>", what);
}
int64_t vb_as_int(VbVal v) {
  switch (v.tag) {
    case VB_INT: return v.v.i;
    case VB_UINT: return (int64_t)v.v.u;
    case VB_FLOAT:
      /* Out of range, or NaN, the cast is undefined; a value is not invented. */
      if (!(v.v.f >= -9223372036854775808.0 && v.v.f < 9223372036854775808.0))
        vb_fail("<runtime>", "a float out of the integer range");
      return (int64_t)v.v.f;
    case VB_BOOL: return v.v.b ? 1 : 0;
    case VB_CHAR: return (int64_t)v.v.c;
    default: vb_fail("<runtime>", "expected an integer"); return 0;
  }
}
uint64_t vb_as_uint(VbVal v) {
  if (v.tag == VB_UINT) return v.v.u;
  if (v.tag == VB_FLOAT) {
    if (!(v.v.f > -1.0 && v.v.f < 18446744073709551616.0))
      vb_fail("<runtime>", "a float out of the unsigned range");
    return (uint64_t)v.v.f;
  }
  return (uint64_t)vb_as_int(v);
}
double vb_as_float(VbVal v) {
  switch (v.tag) {
    case VB_FLOAT: return v.v.f;
    case VB_INT: return (double)v.v.i;
    case VB_UINT: return (double)v.v.u;
    case VB_BOOL: return v.v.b ? 1.0 : 0.0;
    case VB_CHAR: return (double)v.v.c;
    default: vb_fail("<runtime>", "expected a number"); return 0;
  }
}
bool vb_as_bool(VbVal v) { expect(v, VB_BOOL, "expected a Bool"); return v.v.b; }
void *vb_as_ptr(VbVal v) {
  if (v.tag == VB_PTR || v.tag == VB_CSTR) return v.v.p;
  if (v.tag == VB_UNIT) return NULL;
  vb_fail("<runtime>", "expected a pointer");
  return NULL;
}
const char *vb_as_cstr(VbVal v) {
  if (v.tag == VB_CSTR || v.tag == VB_PTR) return (const char *)v.v.p;
  if (v.tag == VB_STR) return ((VbStr *)v.v.p)->p;
  vb_fail("<runtime>", "expected a CStr");
  return NULL;
}
VbStr *vb_as_str(VbVal v) { expect(v, VB_STR, "expected a Str"); return (VbStr *)v.v.p; }
VbVec *vb_as_vec(VbVal v) { expect(v, VB_VEC, "expected a Vec"); return (VbVec *)v.v.p; }
VbObj *vb_as_obj(VbVal v) { expect(v, VB_OBJ, "expected a record or variant"); return (VbObj *)v.v.p; }

/* ----------------------------------------------------------- application */

VbVal vb_apply1(VbVal f, VbVal x) {
  if (f.tag != VB_CLOS) vb_fail("<runtime>", "this value is not a function");
  VbClos *c = (VbClos *)f.v.p;
  if (c->nargs >= c->arity) vb_fail("<runtime>", "applied a function to more arguments than it takes");
  if (c->nargs + 1 == c->arity) {
    /* Saturated: the arguments need to live only for the call. The body reads
       them out of the array, and a closure it builds allocates an array of its
       own, so a frame-local one does, where the arity allows.
       ponytail: the intermediate copies of a partial application are not freed
       here, because the last one is still live when this returns. They die with
       the value that holds them, which is the caller's drop to place. */
    VbVal local[8];
    VbVal *args = c->arity <= 8 ? local : vb_alloc(vals_size(c->arity));
    if (c->nargs) memcpy(args, c->args, c->nargs * sizeof *args);
    args[c->nargs] = x;
    VbVal r = c->fn(args);
    if (args != local) vb_free(args);
    return r;
  }
  VbClos *n = vb_alloc(sizeof(VbClos));
  *n = *c;
  n->args = vb_alloc(vals_size(c->arity));
  if (c->nargs) memcpy(n->args, c->args, c->nargs * sizeof *n->args);
  n->args[n->nargs++] = x;
  return box(VB_CLOS, n);
}

/* Apply `f` to a value the caller was only lent — an element of the `&Vec` that
   `map`, `filter`, `fold`, `each`, `max_by`, `min_by` and `sort_by` walk. A
   function that consumes that parameter frees it, which would free the
   caller's element under it, so it is given a copy to consume instead. */
static VbVal apply_lent(VbVal f, VbVal x) {
  if (f.tag != VB_CLOS) vb_fail("<runtime>", "this value is not a function");
  VbClos *c = (VbClos *)f.v.p;
  bool owns = c->nargs >= 64 || (c->owns >> c->nargs & 1);
  return vb_apply1(f, owns ? vb_dup(x) : x);
}

/* ------------------------------------------------------------ arithmetic
 *
 * Signed arithmetic wraps, through `uint64_t`, where C would leave an overflow
 * undefined: a program that has not proved its bounds gets a wrong number, not
 * a compiler's licence to do anything. The `_checked` forms report it. */

static int64_t wrap(uint64_t x) { return (int64_t)x; }

/* -1, 0 or 1. An unordered pair — NaN — is 0, which is why `vb_eq` does not
   ask `vb_cmp` about floats. */
#define CMP3(x, y) (((x) > (y)) - ((x) < (y)))

static bool either_float(VbVal a, VbVal b) { return a.tag == VB_FLOAT || b.tag == VB_FLOAT; }
static bool either_unsigned(VbVal a, VbVal b) { return a.tag == VB_UINT || b.tag == VB_UINT; }

VbVal vb_add(VbVal a, VbVal b) {
  if (a.tag == VB_STR && b.tag == VB_STR) return vb_concat(a, b);
  if (either_float(a, b)) return vb_float(vb_as_float(a) + vb_as_float(b));
  if (either_unsigned(a, b)) return vb_uint(vb_as_uint(a) + vb_as_uint(b));
  return vb_int(wrap((uint64_t)vb_as_int(a) + (uint64_t)vb_as_int(b)));
}
VbVal vb_sub(VbVal a, VbVal b) {
  if (either_float(a, b)) return vb_float(vb_as_float(a) - vb_as_float(b));
  if (either_unsigned(a, b)) {
    uint64_t x = vb_as_uint(a), y = vb_as_uint(b);
    vb_require(x >= y, "<program>", "unsigned subtraction would wrap");
    return vb_uint(x - y);
  }
  return vb_int(wrap((uint64_t)vb_as_int(a) - (uint64_t)vb_as_int(b)));
}
VbVal vb_mul(VbVal a, VbVal b) {
  if (either_float(a, b)) return vb_float(vb_as_float(a) * vb_as_float(b));
  if (either_unsigned(a, b)) return vb_uint(vb_as_uint(a) * vb_as_uint(b));
  return vb_int(wrap((uint64_t)vb_as_int(a) * (uint64_t)vb_as_int(b)));
}
VbVal vb_div(VbVal a, VbVal b, const char *path) {
  if (either_float(a, b)) {
    double y = vb_as_float(b);
    vb_require(y != 0.0, path, "b != 0.0");
    return vb_float(vb_as_float(a) / y);
  }
  int64_t y = vb_as_int(b);
  vb_require(y != 0, path, "b != 0");
  if (either_unsigned(a, b)) return vb_uint(vb_as_uint(a) / (uint64_t)y);
  int64_t x = vb_as_int(a);
  if (x == INT64_MIN && y == -1) return vb_int(INT64_MIN); /* wraps, as `*` does */
  return vb_int(x / y);
}
VbVal vb_neg(VbVal a) {
  if (a.tag == VB_FLOAT) return vb_float(-a.v.f);
  return vb_int(wrap(0u - (uint64_t)vb_as_int(a)));
}

int vb_cmp(VbVal a, VbVal b) {
  if (a.tag == VB_UNIT && b.tag == VB_UNIT) return 0;
  /* A tuple, a record or a variant orders by constructor, then field by field;
     a vector element by element, then by length. */
  if (a.tag == VB_OBJ && b.tag == VB_OBJ) {
    VbObj *x = a.v.p, *y = b.v.p;
    if (x->tag != y->tag) return CMP3(x->tag, y->tag);
    for (uint32_t i = 0; i < x->n && i < y->n; i++) {
      int c = vb_cmp(x->f[i], y->f[i]);
      if (c) return c;
    }
    return CMP3(x->n, y->n);
  }
  if (a.tag == VB_VEC && b.tag == VB_VEC) {
    VbVec *x = a.v.p, *y = b.v.p;
    for (size_t i = 0; i < x->n && i < y->n; i++) {
      int c = vb_cmp(x->a[i], y->a[i]);
      if (c) return c;
    }
    return CMP3(x->n, y->n);
  }
  if (a.tag == VB_PTR && b.tag == VB_PTR) {
    return CMP3((uintptr_t)a.v.p, (uintptr_t)b.v.p);
  }
  if (a.tag == VB_CSTR && b.tag == VB_CSTR) {
    return CMP3(strcmp(a.v.p ? a.v.p : "", b.v.p ? b.v.p : ""), 0);
  }
  if (a.tag == VB_STR && b.tag == VB_STR) {
    VbStr *x = a.v.p, *y = b.v.p;
    int c = memcmp(x->p, y->p, x->n < y->n ? x->n : y->n);
    return c ? CMP3(c, 0) : CMP3(x->n, y->n);
  }
  if (either_float(a, b)) return CMP3(vb_as_float(a), vb_as_float(b));
  if (either_unsigned(a, b)) return CMP3(vb_as_uint(a), vb_as_uint(b));
  return CMP3(vb_as_int(a), vb_as_int(b));
}

bool vb_eq(VbVal a, VbVal b) {
  /* NaN is equal to nothing, itself included; `vb_cmp` cannot say that. */
  if (either_float(a, b)) return vb_as_float(a) == vb_as_float(b);
  if (a.tag == VB_STR && b.tag == VB_STR) {
    VbStr *x = a.v.p, *y = b.v.p;
    return x->n == y->n && memcmp(x->p, y->p, x->n) == 0;
  }
  if (a.tag == VB_OBJ && b.tag == VB_OBJ) {
    VbObj *x = a.v.p, *y = b.v.p;
    if (x->tag != y->tag || x->n != y->n) return false;
    for (uint32_t i = 0; i < x->n; i++)
      if (!vb_eq(x->f[i], y->f[i])) return false;
    return true;
  }
  if (a.tag == VB_VEC && b.tag == VB_VEC) {
    VbVec *x = a.v.p, *y = b.v.p;
    if (x->n != y->n) return false;
    for (size_t i = 0; i < x->n; i++)
      if (!vb_eq(x->a[i], y->a[i])) return false;
    return true;
  }
  return vb_cmp(a, b) == 0;
}

/* ------------------------------------------------------------ objects */

VbVal vb_field(VbVal o, uint32_t i) {
  VbObj *x = vb_as_obj(o);
  vb_require(i < x->n, "<runtime>", "field index in range");
  return x->f[i];
}
uint32_t vb_tag(VbVal o) { return vb_as_obj(o)->tag; }

/* Uniquely owned update (spec 4.3): mutate in place, no allocation. The
   compiler only emits this where ownership analysis proved the base is not
   shared. */
VbVal vb_set_fields(VbVal o, uint32_t nchanged, const uint32_t *idx, const VbVal *vals) {
  VbObj *x = vb_as_obj(o);
  for (uint32_t i = 0; i < nchanged; i++) x->f[idx[i]] = vals[i];
  return o;
}

VbVal vb_with(VbVal o, uint32_t nchanged, const uint32_t *idx, const VbVal *vals) {
  VbObj *x = vb_as_obj(o);
  VbObj *y = obj_alloc(x->info, x->tag, x->n);
  if (x->n) memcpy(y->f, x->f, x->n * sizeof *y->f);
  for (uint32_t i = 0; i < nchanged; i++) y->f[idx[i]] = vals[i];
  return box(VB_OBJ, y);
}

/* ---------------------------------------------------------------- Vec */

static VbVal wrap_vec(VbVec *w) { return box(VB_VEC, w); }

static VbVec *vec_alloc(size_t cap) {
  VbVec *w = vb_alloc(sizeof(VbVec));
  w->n = 0; w->cap = cap;
  w->a = cap ? vb_alloc(vals_size(cap)) : NULL;
  return w;
}
static void vec_push(VbVec *w, VbVal x) {
  if (w->n == w->cap) {
    size_t cap = w->cap ? w->cap * 2 : 8;
    /* The array is internal to this vector: no other value points at it. */
    w->a = grow(w->a, vals_size(cap));
    w->cap = cap;
  }
  w->a[w->n++] = x;
}
/* Copies of `s`'s elements from `from` up to `to`, appended to `w`. */
static void dup_into(VbVec *w, const VbVec *s, size_t from, size_t to) {
  for (size_t i = from; i < to; i++) vec_push(w, vb_dup(s->a[i]));
}
/* An index, or the obligation `what` refuted at `path`. */
static size_t checked_index(VbVal i, size_t n, const char *path, const char *what) {
  uint64_t k = vb_as_uint(i);
  vb_require(k < n, path, what);
  return (size_t)k;
}

VbVal vb_vec_new(void) { return wrap_vec(vec_alloc(0)); }
VbVal vb_single(VbVal x) { VbVec *w = vec_alloc(1); vec_push(w, x); return wrap_vec(w); }

VbVal vb_vec_lit(uint32_t n, ...) {
  VbVec *w = vec_alloc(n);
  va_list ap; va_start(ap, n);
  for (uint32_t i = 0; i < n; i++) vec_push(w, va_arg(ap, VbVal));
  va_end(ap);
  return wrap_vec(w);
}

/* The operations that take `&Vec` — `rev`, `filter`, `sort_by`, `take`,
 * `drop`, `concat_vec` — cannot move the elements: the caller still owns the
 * source and will free it, so each element is copied with `vb_dup` and no two
 * vectors point at one value (docs/aliasing-audit.md, gap 3). That copy is what
 * makes `vb_dispose` able to be deep.
 *
 * `push`, `set`, `insert` and `remove` take the collection by value, but own.rs
 * picks these copying forms exactly when that value may still be part of
 * another one (`push (get vv 0) x`), so they copy too and free nothing. The
 * `_owned` forms below are the ones for a collection the frame holds alone.
 *
 * ponytail: the copy is unconditional. It is dead work when the source dies at
 * the call, and the drop table already computes last use — eliding it there is
 * Open decision 5 of docs/static-drop-roadmap.md, and it needs a measurement
 * first. */
VbVal vb_push(VbVal v, VbVal x) {
  VbVec *s = vb_as_vec(v);
  VbVec *w = vec_alloc(s->n + 1);
  dup_into(w, s, 0, s->n);
  vec_push(w, x);
  return wrap_vec(w);
}

/* The same two operations on a vector the caller owns alone (own.rs decides):
 * the spine grows or is written in place, amortised O(1) per push rather than
 * a copy of every element. */
VbVal vb_push_owned(VbVal v, VbVal x) {
  vec_push(vb_as_vec(v), x);
  return v;
}
VbVal vb_set_owned(VbVal v, VbVal i, VbVal x, const char *path) {
  VbVec *s = vb_as_vec(v);
  s->a[checked_index(i, s->n, path, "i < len xs")] = x;
  return v;
}

VbVal vb_byte_at(VbVal s, VbVal i, const char *path) {
  VbStr *x = vb_as_str(s);
  return vb_uint((unsigned char)x->p[checked_index(i, x->n, path, "i < len s")]);
}
VbVal vb_get(VbVal v, VbVal i, const char *path) {
  VbVec *s = vb_as_vec(v);
  return s->a[checked_index(i, s->n, path, "i < len xs")];
}
VbVal vb_set(VbVal v, VbVal i, VbVal x, const char *path) {
  VbVec *s = vb_as_vec(v);
  size_t k = checked_index(i, s->n, path, "i < len xs");
  VbVec *w = vec_alloc(s->n);
  for (size_t j = 0; j < s->n; j++) vec_push(w, j == k ? x : vb_dup(s->a[j]));
  return wrap_vec(w);
}
VbVal vb_len(VbVal v) {
  if (v.tag == VB_STR) return vb_uint(vb_as_str(v)->n);
  return vb_uint(vb_as_vec(v)->n);
}
VbVal vb_map(VbVal f, VbVal v) {
  VbVec *s = vb_as_vec(v);
  VbVec *w = vec_alloc(s->n);
  for (size_t i = 0; i < s->n; i++) vec_push(w, apply_lent(f, s->a[i]));
  return wrap_vec(w);
}
VbVal vb_filter(VbVal f, VbVal v) {
  VbVec *s = vb_as_vec(v);
  VbVec *w = vec_alloc(s->n);
  for (size_t i = 0; i < s->n; i++)
    if (vb_as_bool(apply_lent(f, s->a[i]))) vec_push(w, vb_dup(s->a[i]));
  return wrap_vec(w);
}
VbVal vb_fold(VbVal f, VbVal z, VbVal v) {
  VbVec *s = vb_as_vec(v);
  for (size_t i = 0; i < s->n; i++) {
    /* The partial application holding `z` is this loop's alone. Its drop is
       shallow, so `z` itself goes on to the call. */
    VbVal g = vb_apply1(f, z);
    z = apply_lent(g, s->a[i]);
    vb_dispose(g);
  }
  return z;
}
VbVal vb_each(VbVal f, VbVal v) {
  VbVec *s = vb_as_vec(v);
  for (size_t i = 0; i < s->n; i++) apply_lent(f, s->a[i]);
  return vb_unit();
}
VbVal vb_sum(VbVal v) {
  VbVec *s = vb_as_vec(v);
  if (s->n == 0) return vb_int(0);
  VbVal acc = s->a[0];
  for (size_t i = 1; i < s->n; i++) {
    VbVal next = vb_add(acc, s->a[i]);
    /* A partial sum of strings is a fresh string; the first is an element. */
    if (i > 1) vb_dispose(acc);
    acc = next;
  }
  return acc;
}
/* The first element whose key compares strictly beyond every earlier one in
   the direction of `sign`: +1 for the largest, -1 for the smallest. */
static VbVal extreme_by(VbVal f, VbVal v, const char *path, int sign) {
  VbVec *s = vb_as_vec(v);
  vb_require(s->n > 0, path, "len xs > 0");
  size_t best = 0; VbVal bk = apply_lent(f, s->a[0]);
  for (size_t i = 1; i < s->n; i++) {
    VbVal k = apply_lent(f, s->a[i]);
    if (vb_cmp(k, bk) * sign > 0) { best = i; bk = k; }
  }
  return s->a[best];
}
VbVal vb_max_by(VbVal f, VbVal v, const char *path) { return extreme_by(f, v, path, 1); }
VbVal vb_min_by(VbVal f, VbVal v, const char *path) { return extreme_by(f, v, path, -1); }
/* Stable merge sort on (key, element) pairs: each key is computed once, where
   the insertion sort it replaces called `f` twice per comparison, O(n²) times. */
typedef struct { VbVal key, val; } KeyVal;
static void merge_sort(KeyVal *a, KeyVal *tmp, size_t n) {
  if (n < 2) return;
  size_t h = n / 2;
  merge_sort(a, tmp, h);
  merge_sort(a + h, tmp, n - h);
  size_t i = 0, j = h, k = 0;
  /* `<=` takes from the left on a tie, which is what keeps it stable. */
  while (i < h && j < n) tmp[k++] = vb_cmp(a[i].key, a[j].key) <= 0 ? a[i++] : a[j++];
  while (i < h) tmp[k++] = a[i++];
  while (j < n) tmp[k++] = a[j++];
  memcpy(a, tmp, n * sizeof *a);
}
VbVal vb_sort_by(VbVal f, VbVal v) {
  VbVec *s = vb_as_vec(v);
  KeyVal *kv = vb_alloc(2 * vals_size(s->n));
  KeyVal *tmp = vb_alloc(2 * vals_size(s->n));
  for (size_t i = 0; i < s->n; i++) { kv[i].val = s->a[i]; kv[i].key = apply_lent(f, s->a[i]); }
  merge_sort(kv, tmp, s->n);
  VbVec *w = vec_alloc(s->n);
  for (size_t i = 0; i < s->n; i++) vec_push(w, vb_dup(kv[i].val));
  vb_free(kv);
  vb_free(tmp);
  return wrap_vec(w);
}
VbVal vb_rev(VbVal v) {
  VbVec *s = vb_as_vec(v);
  VbVec *w = vec_alloc(s->n);
  for (size_t i = s->n; i > 0; i--) vec_push(w, vb_dup(s->a[i - 1]));
  return wrap_vec(w);
}
VbVal vb_concat_vec(VbVal a, VbVal b) {
  VbVec *x = vb_as_vec(a), *y = vb_as_vec(b);
  VbVec *w = vec_alloc(x->n + y->n);
  dup_into(w, x, 0, x->n);
  dup_into(w, y, 0, y->n);
  return wrap_vec(w);
}
VbVal vb_take(VbVal n, VbVal v) {
  VbVec *s = vb_as_vec(v);
  uint64_t k = vb_as_uint(n);
  if (k > s->n) k = s->n;
  VbVec *w = vec_alloc((size_t)k);
  dup_into(w, s, 0, (size_t)k);
  return wrap_vec(w);
}
VbVal vb_drop(VbVal n, VbVal v) {
  VbVec *s = vb_as_vec(v);
  uint64_t k = vb_as_uint(n);
  if (k > s->n) k = s->n;
  VbVec *w = vec_alloc(s->n - (size_t)k);
  dup_into(w, s, (size_t)k, s->n);
  return wrap_vec(w);
}
VbVal vb_range(VbVal a, VbVal b) {
  uint64_t lo = vb_as_uint(a), hi = vb_as_uint(b);
  if (hi > lo && hi - lo > SIZE_MAX) oom();
  VbVec *w = vec_alloc(hi > lo ? (size_t)(hi - lo) : 0);
  for (uint64_t i = lo; i < hi; i++) vec_push(w, vb_uint(i));
  return wrap_vec(w);
}

/* ---------------------------------------------------------------- Str */

/* A code point as UTF-8, into `out`; returns the byte count. One that is not a
   scalar value — a surrogate, or past U+10FFFF — is U+FFFD. */
static size_t utf8(uint32_t c, char *out) {
  if (c < 0x80) { out[0] = (char)c; return 1; }
  if (c < 0x800) {
    out[0] = (char)(0xC0 | c >> 6); out[1] = (char)(0x80 | (c & 0x3F));
    return 2;
  }
  if (c > 0x10FFFF || (c >= 0xD800 && c <= 0xDFFF)) c = 0xFFFD;
  if (c < 0x10000) {
    out[0] = (char)(0xE0 | c >> 12); out[1] = (char)(0x80 | (c >> 6 & 0x3F));
    out[2] = (char)(0x80 | (c & 0x3F));
    return 3;
  }
  out[0] = (char)(0xF0 | c >> 18); out[1] = (char)(0x80 | (c >> 12 & 0x3F));
  out[2] = (char)(0x80 | (c >> 6 & 0x3F)); out[3] = (char)(0x80 | (c & 0x3F));
  return 4;
}

VbVal vb_lines(VbVal s) {
  VbStr *x = vb_as_str(s);
  VbVec *w = vec_alloc(8);
  size_t start = 0;
  for (size_t i = 0; i < x->n; i++) {
    if (x->p[i] == '\n') {
      size_t end = i;
      if (end > start && x->p[end - 1] == '\r') end--;
      vec_push(w, vb_str(x->p + start, end - start));
      start = i + 1;
    }
  }
  if (start < x->n) {
    size_t end = x->n;
    if (x->p[end - 1] == '\r') end--;
    vec_push(w, vb_str(x->p + start, end - start));
  }
  return wrap_vec(w);
}
/* Copy a value in depth, so the copy owns everything it points at and both can
   be used, and later freed, without the other. Spec 4.4 names "return a copy"
   as one of the three answers to the absence of lifetimes; a shallow copy is
   not one of them, because the two values would share what they point at and
   the affine checker would be reasoning about one value where there are two.

   `VB_CSTR` and `VB_PTR` stay shallow: the memory is C's, its extent is not
   known here, and copying the pointer is the only operation with a meaning. A
   copy containing one therefore still aliases, which is the drop suppression
   the frontend already makes for anything that reaches C (`src/escape.rs`);
   deep drop has to keep making it.

   The recursion terminates because owned data is acyclic: 16.6 puts graphs and
   arbitrary sharing in an arena, not in a value. */
VbVal vb_dup(VbVal v) {
  switch (v.tag) {
    case VB_STR: { VbStr *x = v.v.p; return vb_str(x->p, x->n); }
    case VB_VEC: {
      VbVec *x = v.v.p;
      VbVec *w = vec_alloc(x->n);
      dup_into(w, x, 0, x->n);
      return wrap_vec(w);
    }
    case VB_OBJ: {
      VbObj *x = v.v.p;
      VbObj *o = obj_alloc(x->info, x->tag, x->n);
      for (uint32_t i = 0; i < x->n; i++) o->f[i] = vb_dup(x->f[i]);
      return box(VB_OBJ, o);
    }
    case VB_CLOS: {
      VbClos *x = v.v.p;
      VbClos *c = vb_alloc(sizeof(VbClos));
      *c = *x;
      c->args = x->arity ? vb_alloc(vals_size(x->arity)) : NULL;
      /* Shallow, as `vb_dispose` is for a closure: a deep copy of captures
         that nothing frees would only leak them. */
      if (x->nargs) memcpy(c->args, x->args, x->nargs * sizeof *c->args);
      return box(VB_CLOS, c);
    }
    /* A scalar is already a copy; a C pointer is not ours to duplicate. */
    default: return v;
  }
}
VbVal vb_concat(VbVal a, VbVal b) {
  VbStr *x = vb_as_str(a), *y = vb_as_str(b);
  char *buf = vb_alloc(x->n + y->n + 1);
  memcpy(buf, x->p, x->n);
  memcpy(buf + x->n, y->p, y->n);
  return str_take(buf, x->n + y->n);
}
static bool is_space(char c) { return c == ' ' || c == '\t' || c == '\n' || c == '\r'; }
VbVal vb_trim(VbVal s) {
  VbStr *x = vb_as_str(s);
  size_t i = 0, j = x->n;
  while (i < j && is_space(x->p[i])) i++;
  while (j > i && is_space(x->p[j - 1])) j--;
  return vb_str(x->p + i, j - i);
}
VbVal vb_starts_with(VbVal s, VbVal p) {
  VbStr *x = vb_as_str(s), *y = vb_as_str(p);
  return vb_bool(y->n <= x->n && memcmp(x->p, y->p, y->n) == 0);
}
/* The first index at or after `from` where `y` occurs in `x`, or SIZE_MAX.
   `memchr` jumps to each candidate first byte, so a scan costs a comparison
   only where the needle could start. An empty needle is found at `from`.
   ponytail: still O(n*m) on adversarial input; two-way if a profile blames it. */
static size_t str_find(const VbStr *x, const VbStr *y, size_t from) {
  if (y->n == 0) return from <= x->n ? from : SIZE_MAX;
  if (y->n > x->n) return SIZE_MAX;
  size_t last = x->n - y->n;
  for (size_t i = from; i <= last;) {
    const char *c = memchr(x->p + i, y->p[0], last - i + 1);
    if (!c) return SIZE_MAX;
    i = (size_t)(c - x->p);
    if (memcmp(c, y->p, y->n) == 0) return i;
    i++;
  }
  return SIZE_MAX;
}
/* The separator is a character, so it is matched as its UTF-8 bytes, not cut
   down to one. */
VbVal vb_split(VbVal c, VbVal s) {
  VbStr *x = vb_as_str(s);
  char buf[4];
  VbStr sep = {utf8((uint32_t)vb_as_uint(c), buf), buf};
  VbVec *w = vec_alloc(4);
  size_t start = 0;
  for (size_t i; (i = str_find(x, &sep, start)) != SIZE_MAX; start = i + sep.n)
    vec_push(w, vb_str(x->p + start, i - start));
  vec_push(w, vb_str(x->p + start, x->n - start));
  return wrap_vec(w);
}
VbVal vb_contains(VbVal s, VbVal p) {
  return vb_bool(str_find(vb_as_str(s), vb_as_str(p), 0) != SIZE_MAX);
}
VbVal vb_to_cstr(VbVal s) { return vb_cstr_val(vb_as_str(s)->p); }
VbVal vb_from_cstr(VbVal p) { return vb_strz((const char *)vb_as_ptr(p)); }
VbVal vb_chr(VbVal c) { char b[4]; return vb_str(b, utf8((uint32_t)vb_as_uint(c), b)); }
VbVal vb_byte_str(VbVal b) { char c = (char)vb_as_uint(b); return vb_str(&c, 1); }
/* A substring is a copy, never a pointer into the argument: the prelude may
   not hand out an interior pointer as an owned value (docs/aliasing-audit.md). */
VbVal vb_slice(VbVal i, VbVal j, VbVal s) {
  VbStr *x = vb_as_str(s);
  uint64_t a = vb_as_uint(i), b = vb_as_uint(j);
  if (a > b || b > x->n) return vb_none();
  return vb_some(vb_str(x->p + a, (size_t)(b - a)));
}
VbVal vb_index_of(VbVal s, VbVal p) {
  size_t i = str_find(vb_as_str(s), vb_as_str(p), 0);
  return i == SIZE_MAX ? vb_none() : vb_some(vb_uint(i));
}
/* A growable byte buffer for `show`, `fmt` and `replace`. It keeps a byte spare,
   so the string it becomes takes the buffer rather than a copy of it. */
typedef struct { char *p; size_t n, cap; } Sb;
static void sb_push(Sb *acc, const char *s, size_t n) {
  if (acc->n + n + 1 > acc->cap) {
    size_t cap = acc->cap ? acc->cap : 64;
    while (cap < acc->n + n + 1) {
      if (cap > SIZE_MAX / 2) oom();
      cap *= 2;
    }
    acc->p = grow(acc->p, cap);
    acc->cap = cap;
  }
  if (n) memcpy(acc->p + acc->n, s, n);
  acc->n += n;
}
static void sb_puts(Sb *acc, const char *s) { if (s) sb_push(acc, s, strlen(s)); }
static VbVal sb_done(Sb *acc) {
  sb_push(acc, "", 0);
  return str_take(acc->p, acc->n);
}

/* Non-overlapping, left to right. An empty needle replaces nothing. */
VbVal vb_replace(VbVal s, VbVal from, VbVal to) {
  VbStr *x = vb_as_str(s), *f = vb_as_str(from), *t = vb_as_str(to);
  if (f->n == 0) return vb_str(x->p, x->n);
  Sb acc = {0};
  size_t done = 0; /* the input already copied */
  for (size_t i; (i = str_find(x, f, done)) != SIZE_MAX; done = i + f->n) {
    sb_push(&acc, x->p + done, i - done);
    sb_push(&acc, t->p, t->n);
  }
  sb_push(&acc, x->p + done, x->n - done);
  return sb_done(&acc);
}
/* ponytail: ASCII only. Upgrade path is a UTF-8 case table, when a program that
   needs one exists. */
VbVal vb_lower(VbVal s) {
  VbStr *x = vb_as_str(s);
  VbVal r = vb_str(x->p, x->n);
  char *p = ((VbStr *)r.v.p)->p;
  for (size_t i = 0; i < x->n; i++)
    if (p[i] >= 'A' && p[i] <= 'Z') p[i] = (char)(p[i] - 'A' + 'a');
  return r;
}

static void show_into(Sb *acc, VbVal v) {
  char tmp[64];
  switch (v.tag) {
    case VB_UNIT: sb_puts(acc, "()"); break;
    case VB_INT: sb_push(acc, tmp, (size_t)snprintf(tmp, sizeof tmp, "%lld", (long long)v.v.i)); break;
    case VB_UINT: sb_push(acc, tmp, (size_t)snprintf(tmp, sizeof tmp, "%llu", (unsigned long long)v.v.u)); break;
    case VB_FLOAT: sb_push(acc, tmp, (size_t)snprintf(tmp, sizeof tmp, "%g", v.v.f)); break;
    case VB_BOOL: sb_puts(acc, v.v.b ? "True" : "False"); break;
    case VB_CHAR: sb_push(acc, tmp, utf8(v.v.c, tmp)); break;
    case VB_STR: { VbStr *s = (VbStr *)v.v.p; sb_push(acc, s->p, s->n); break; }
    case VB_CSTR: sb_puts(acc, v.v.p); break;
    case VB_PTR: sb_push(acc, tmp, (size_t)snprintf(tmp, sizeof tmp, "0x%llx", (unsigned long long)(uintptr_t)v.v.p)); break;
    case VB_CLOS: sb_puts(acc, "<"); sb_puts(acc, ((VbClos *)v.v.p)->name); sb_puts(acc, ">"); break;
    case VB_VEC: {
      VbVec *s = (VbVec *)v.v.p;
      sb_puts(acc, "[");
      for (size_t i = 0; i < s->n; i++) { if (i) sb_puts(acc, ", "); show_into(acc, s->a[i]); }
      sb_puts(acc, "]");
      break;
    }
    case VB_OBJ: {
      VbObj *o = (VbObj *)v.v.p;
      if (o->info && o->info->fields) {
        sb_puts(acc, "{");
        for (uint32_t i = 0; i < o->n; i++) {
          if (i) sb_puts(acc, ", ");
          sb_puts(acc, o->info->fields[i]);
          sb_puts(acc, "=");
          show_into(acc, o->f[i]);
        }
        sb_puts(acc, "}");
      } else {
        sb_puts(acc, o->info ? o->info->name : "?");
        for (uint32_t i = 0; i < o->n; i++) { sb_puts(acc, " "); show_into(acc, o->f[i]); }
      }
      break;
    }
    default: sb_puts(acc, "?");
  }
}

VbVal vb_show(VbVal v) { Sb acc = {0}; show_into(&acc, v); return sb_done(&acc); }

VbVal vb_fmt(VbVal f, uint32_t n, ...) {
  VbStr *s = vb_as_str(f);
  va_list ap; va_start(ap, n);
  Sb acc = {0};
  uint32_t k = 0;
  size_t run = 0; /* start of the literal text not yet copied */
  for (size_t i = 0; i + 1 < s->n; i++) {
    if (s->p[i] == '{' && s->p[i + 1] == '}') {
      sb_push(&acc, s->p + run, i - run);
      if (k < n) { show_into(&acc, va_arg(ap, VbVal)); k++; } else sb_puts(&acc, "{}");
      i++;
      run = i + 1;
    }
  }
  sb_push(&acc, s->p + run, s->n - run);
  va_end(ap);
  return sb_done(&acc);
}

/* -------------------------------------------------------- conversions */

VbVal vb_to_f64(VbVal v) { return vb_float(vb_as_float(v)); }
VbVal vb_to_f32(VbVal v) { return vb_float((double)(float)vb_as_float(v)); }
VbVal vb_to_signed(VbVal v, int bits) {
  int64_t x = vb_as_int(v);
  switch (bits) {
    case 8: return vb_int((int8_t)x);
    case 16: return vb_int((int16_t)x);
    case 32: return vb_int((int32_t)x);
    default: return vb_int(x);
  }
}
VbVal vb_to_unsigned(VbVal v, int bits) {
  uint64_t x = vb_as_uint(v);
  switch (bits) {
    case 8: return vb_uint((uint8_t)x);
    case 16: return vb_uint((uint16_t)x);
    case 32: return vb_uint((uint32_t)x);
    default: return vb_uint(x);
  }
}

/* ------------------------------------------------------------ Res / Opt */

const VbInfo vb_info_Ok = {"Ok", 1, NULL};
const VbInfo vb_info_Er = {"Er", 1, NULL};
const VbInfo vb_info_Some = {"Some", 1, NULL};
const VbInfo vb_info_None = {"None", 0, NULL};
const VbInfo vb_info_Overflow = {"Overflow", 0, NULL};
const VbInfo vb_info_DivZero = {"DivZero", 0, NULL};
const VbInfo vb_info_OutOfBounds = {"OutOfBounds", 0, NULL};

VbVal vb_ok(VbVal x) { return vb_obj(&vb_info_Ok, 0, 1, x); }
VbVal vb_er(VbVal x) { return vb_obj(&vb_info_Er, 1, 1, x); }
VbVal vb_some(VbVal x) { return vb_obj(&vb_info_Some, 0, 1, x); }
VbVal vb_none(void) { return vb_obj(&vb_info_None, 1, 0); }

/* ------------------------------------------------------------------ Dict

   An association vector: a `VB_VEC` whose elements are two-field objects. It is
   not a new tag, which is the point — `vb_dispose`, `vb_dup`, `vb_len` and
   `show` already do the right thing for a vector of objects, and the
   representation can change without any of them knowing.

   Keys are compared with `vb_eq`, the language's own equality, so a key is
   whatever the type system already lets you write one of.

   ponytail: lookup is a linear scan, so a dictionary of n entries costs O(n)
   and building one costs O(n^2). Upgrade path: a hash table behind the same
   five functions, which needs a hash for every tag that `vb_eq` compares — do
   it when a program is slow, not because the complexity is embarrassing. */

static const char *const vb_entry_fields[] = {"key", "value"};
static const VbInfo vb_info_entry = {"entry", 2, vb_entry_fields};

VbVal vb_dict(void) { return wrap_vec(vec_alloc(0)); }

/* The index of `k`, or `d->n` when it is not there. */
static size_t dict_find(VbVec *d, VbVal k) {
  for (size_t i = 0; i < d->n; i++)
    if (vb_eq(((VbObj *)d->a[i].v.p)->f[0], k)) return i;
  return d->n;
}

/* The copying forms, for a dictionary that may still be part of another value:
   every entry but the displaced one is copied, and nothing is freed. */
static VbVec *dict_copy_without(VbVec *s, size_t at, size_t extra) {
  VbVec *w = vec_alloc(s->n + extra);
  for (size_t i = 0; i < s->n; i++)
    if (i != at) vec_push(w, vb_dup(s->a[i]));
  return w;
}
VbVal vb_insert(VbVal d, VbVal k, VbVal v) {
  VbVec *s = vb_as_vec(d);
  VbVec *w = dict_copy_without(s, dict_find(s, k), 1);
  vec_push(w, vb_obj(&vb_info_entry, 0, 2, k, v));
  return wrap_vec(w);
}
VbVal vb_remove(VbVal d, VbVal k) {
  VbVec *s = vb_as_vec(d);
  return wrap_vec(dict_copy_without(s, dict_find(s, k), 0));
}

/* The in-place forms, for a dictionary the frame holds alone (own.rs decides):
   the entry a key displaces is nobody's after this, so it goes, and the rest
   keep their order. */
static void dict_unlink(VbVec *s, size_t at) {
  if (at == s->n) return;
  vb_dispose(s->a[at]);
  memmove(s->a + at, s->a + at + 1, (s->n - at - 1) * sizeof *s->a);
  s->n--;
}
VbVal vb_insert_owned(VbVal d, VbVal k, VbVal v) {
  VbVec *s = vb_as_vec(d);
  dict_unlink(s, dict_find(s, k));
  vec_push(s, vb_obj(&vb_info_entry, 0, 2, k, v));
  return d;
}
VbVal vb_remove_owned(VbVal d, VbVal k) {
  VbVec *s = vb_as_vec(d);
  dict_unlink(s, dict_find(s, k));
  return d;
}

/* Borrows, so the value comes back as a copy: handing out the entry's own
   pointer would be an interior pointer into a dictionary the caller still owns
   (docs/aliasing-audit.md, gap 2). */
VbVal vb_lookup(VbVal d, VbVal k) {
  VbVec *s = vb_as_vec(d);
  size_t at = dict_find(s, k);
  if (at == s->n) return vb_none();
  return vb_some(vb_dup(((VbObj *)s->a[at].v.p)->f[1]));
}

VbVal vb_keys(VbVal d) {
  VbVec *s = vb_as_vec(d);
  VbVec *w = vec_alloc(s->n);
  for (size_t i = 0; i < s->n; i++) vec_push(w, vb_dup(((VbObj *)s->a[i].v.p)->f[0]));
  return wrap_vec(w);
}

VbVal vb_seq(VbVal v) {
  VbVec *s = vb_as_vec(v);
  VbVec *w = vec_alloc(s->n);
  for (size_t i = 0; i < s->n; i++) {
    VbObj *o = vb_as_obj(s->a[i]);
    if (o->tag != 0) { vb_free(w->a); vb_free(w); return s->a[i]; } /* the first Er wins */
    vec_push(w, o->f[0]);
  }
  return vb_ok(wrap_vec(w));
}

/* Digits, with a `-` in front for a signed type, and nothing else: `strtoll`
   alone would also take leading space and a `+`, clamp what is out of range,
   and let `strtoull` wrap " -5" to a huge number. `bits` bounds the result. */
VbVal vb_parse_int(VbVal s, int sign, int bits) {
  VbStr *x = vb_as_str(s);
  const char *p = x->p;
  bool neg = sign && x->n > 0 && p[0] == '-';
  if (x->n == (size_t)neg || p[neg] < '0' || p[neg] > '9') return vb_none();
  char *end = NULL;
  errno = 0;
  if (sign) {
    long long r = strtoll(p, &end, 10);
    int64_t lim = bits >= 64 ? INT64_MAX : (INT64_C(1) << (bits - 1)) - 1;
    if (errno || end != p + x->n || r > lim || r < -lim - 1) return vb_none();
    return vb_some(vb_int((int64_t)r));
  }
  unsigned long long r = strtoull(p, &end, 10);
  uint64_t lim = bits >= 64 ? UINT64_MAX : (UINT64_C(1) << bits) - 1;
  if (errno || end != p + x->n || r > lim) return vb_none();
  return vb_some(vb_uint((uint64_t)r));
}
VbVal vb_parse_f64(VbVal s) {
  VbStr *x = vb_as_str(s);
  if (x->n == 0 || isspace((unsigned char)x->p[0])) return vb_none();
  char *end = NULL;
  double r = strtod(x->p, &end);
  if (end != x->p + x->n) return vb_none();
  return vb_some(vb_float(r));
}

/* ---------------------------------------------------------------- Math */

VbVal vb_abs(VbVal a) {
  if (a.tag == VB_FLOAT) return vb_float(fabs(a.v.f));
  if (a.tag == VB_UINT) return a;
  int64_t x = vb_as_int(a);
  return vb_int(x < 0 ? wrap(0u - (uint64_t)x) : x);
}
/* Bit operations are functions, not operators: `&` is the borrow sigil and `|`
   separates match arms, and the two spellings that were left would have brought
   a precedence table with them — the one where `a band b == c` silently means
   `a band (b == c)` in C. A call has no precedence to get wrong.

   A bit pattern is an integer. A float here is a mistake rather than a value to
   coerce, so it is refused instead of truncated. */
static uint64_t bits(VbVal v, const char *what) {
  vb_require(v.tag != VB_FLOAT, "<program>", what);
  return vb_as_uint(v);
}

/* Unsigned wins, as it does for arithmetic: the result of a bit operation on a
   `U64` is a `U64`. */
static VbVal as_wide(VbVal a, VbVal b, uint64_t x) {
  return either_unsigned(a, b) ? vb_uint(x) : vb_int(wrap(x));
}

VbVal vb_band(VbVal a, VbVal b) {
  return as_wide(a, b, bits(a, "band needs an integer") & bits(b, "band needs an integer"));
}
VbVal vb_bor(VbVal a, VbVal b) {
  return as_wide(a, b, bits(a, "bor needs an integer") | bits(b, "bor needs an integer"));
}
VbVal vb_bxor(VbVal a, VbVal b) {
  return as_wide(a, b, bits(a, "bxor needs an integer") ^ bits(b, "bxor needs an integer"));
}
VbVal vb_bnot(VbVal a) {
  return as_wide(a, a, ~bits(a, "bnot needs an integer"));
}

/* A shift wider than the word is zero — or, for a signed right shift of a
   negative value, all ones. C leaves both undefined, which is not an answer a
   generated program can be given. */
VbVal vb_shl(VbVal a, VbVal n) {
  uint64_t k = bits(n, "a shift count is an integer");
  uint64_t x = bits(a, "shl needs an integer");
  return as_wide(a, a, k >= 64 ? 0 : x << k);
}

VbVal vb_shr(VbVal a, VbVal n) {
  uint64_t k = bits(n, "a shift count is an integer");
  if (a.tag == VB_UINT) {
    uint64_t x = a.v.u;
    return vb_uint(k >= 64 ? 0 : x >> k);
  }
  /* Signed: arithmetic, so the sign is kept. */
  int64_t x = (int64_t)bits(a, "shr needs an integer");
  if (k >= 64) return vb_int(x < 0 ? -1 : 0);
  return vb_int(x >> k);
}

/* The inverse of `chr`: a character's code point. */
VbVal vb_ord(VbVal c) {
  return vb_uint(vb_as_uint(c));
}

VbVal vb_min(VbVal a, VbVal b) { return vb_cmp(a, b) <= 0 ? a : b; }
VbVal vb_max(VbVal a, VbVal b) { return vb_cmp(a, b) >= 0 ? a : b; }
VbVal vb_sqrt(VbVal a) { return vb_float(sqrt(vb_as_float(a))); }
VbVal vb_pow(VbVal a, VbVal b) { return vb_float(pow(vb_as_float(a), vb_as_float(b))); }
VbVal vb_floor(VbVal a) { return vb_float(floor(vb_as_float(a))); }
/* The shortest of 15 or 17 significant digits that reads back as the same double. */
VbVal vb_show_exact(VbVal a) {
  double d = vb_as_float(a);
  char buf[32];
  int n = snprintf(buf, sizeof buf, "%.15g", d);
  if (strtod(buf, NULL) != d) n = snprintf(buf, sizeof buf, "%.17g", d);
  return vb_str(buf, (size_t)n);
}

/* ------------------------------------------------------------------ IO */

static int g_argc = 0;
static char **g_argv = NULL;
void vb_set_args(int argc, char **argv) { g_argc = argc; g_argv = argv; }

/* All of a stream, to its end. No `ftell`: a pipe, `/dev/stdin` or a FIFO has
   no size to ask for, and `long` is 32 bits on Windows.
   ponytail: doubling buffer with a copy on each growth, so peak memory is about
   twice the input. Stream it if a program ever has to filter more than it can
   hold. */
static VbVal read_all(FILE *fh, const char *what) {
  size_t cap = 65536, n = 0;
  char *buf = vb_alloc(cap);
  for (;;) {
    n += fread(buf + n, 1, cap - n, fh);
    if (n < cap) break; /* short read: end of file, or an error */
    if (cap > SIZE_MAX / 2) oom();
    cap *= 2;
    buf = grow(buf, cap);
  }
  if (ferror(fh)) { fprintf(stderr, "✗ read ⊨ cannot read %s\n", what); exit(66); }
  return str_take(buf, n);
}
VbVal vb_read(VbVal path) {
  const char *p = vb_as_str(path)->p;
  FILE *fh = fopen(p, "rb");
  if (!fh) { fprintf(stderr, "✗ read ⊨ cannot open %s\n", p); exit(66); }
  VbVal r = read_all(fh, p);
  fclose(fh);
  return r;
}
VbVal vb_read_stdin(void) { return read_all(stdin, "standard input"); }
VbVal vb_write(VbVal path, VbVal data) {
  const char *p = vb_as_str(path)->p;
  VbStr *d = vb_as_str(data);
  FILE *fh = fopen(p, "wb");
  if (!fh) { fprintf(stderr, "✗ write ⊨ cannot open %s\n", p); exit(73); }
  /* A full disk shows at `fwrite` or only at `fclose`; either way the file is
     not what the program wrote, which is not a success to report. */
  bool ok = fwrite(d->p, 1, d->n, fh) == d->n;
  if (fclose(fh) != 0 || !ok) { fprintf(stderr, "✗ write ⊨ cannot write %s\n", p); exit(73); }
  return vb_unit();
}
VbVal vb_out(VbVal s) { VbStr *x = vb_as_str(s); fwrite(x->p, 1, x->n, stdout); fputc('\n', stdout); return vb_unit(); }
VbVal vb_warn(VbVal s) { VbStr *x = vb_as_str(s); fwrite(x->p, 1, x->n, stderr); fputc('\n', stderr); return vb_unit(); }
VbVal vb_argv(void) {
  VbVec *w = vec_alloc((size_t)(g_argc > 0 ? g_argc : 0));
  for (int i = 0; i < g_argc; i++) vec_push(w, vb_strz(g_argv[i]));
  return wrap_vec(w);
}
VbVal vb_exit(VbVal code) { exit((int)vb_as_int(code)); }

/* ------------------------------------------------------------- Checked */

/* Plain C99 overflow tests: the `__builtin_*_overflow` family is GCC and
   Clang's, and the emitted C is meant for MSVC too. */
static bool add_overflows(int64_t x, int64_t y) {
  return y > 0 ? x > INT64_MAX - y : x < INT64_MIN - y;
}
static bool sub_overflows(int64_t x, int64_t y) {
  return y < 0 ? x > INT64_MAX + y : x < INT64_MIN + y;
}
static bool mul_overflows(int64_t x, int64_t y) {
  if (x == 0 || y == 0) return false;
  if (x > 0) return y > 0 ? x > INT64_MAX / y : y < INT64_MIN / x;
  return y > 0 ? x < INT64_MIN / y : x < INT64_MAX / y;
}
static VbVal fault(const VbInfo *info, uint32_t tag) { return vb_er(vb_obj(info, tag, 0)); }
static VbVal overflow(void) { return fault(&vb_info_Overflow, 0); }

VbVal vb_add_checked(VbVal a, VbVal b) {
  if (either_float(a, b)) return vb_ok(vb_float(vb_as_float(a) + vb_as_float(b)));
  if (either_unsigned(a, b)) {
    uint64_t x = vb_as_uint(a), y = vb_as_uint(b);
    return x > UINT64_MAX - y ? overflow() : vb_ok(vb_uint(x + y));
  }
  int64_t x = vb_as_int(a), y = vb_as_int(b);
  return add_overflows(x, y) ? overflow() : vb_ok(vb_int(x + y));
}
VbVal vb_sub_checked(VbVal a, VbVal b) {
  if (either_float(a, b)) return vb_ok(vb_float(vb_as_float(a) - vb_as_float(b)));
  if (either_unsigned(a, b)) {
    uint64_t x = vb_as_uint(a), y = vb_as_uint(b);
    return x < y ? overflow() : vb_ok(vb_uint(x - y));
  }
  int64_t x = vb_as_int(a), y = vb_as_int(b);
  return sub_overflows(x, y) ? overflow() : vb_ok(vb_int(x - y));
}
VbVal vb_mul_checked(VbVal a, VbVal b) {
  if (either_float(a, b)) return vb_ok(vb_float(vb_as_float(a) * vb_as_float(b)));
  if (either_unsigned(a, b)) {
    uint64_t x = vb_as_uint(a), y = vb_as_uint(b);
    return x != 0 && y > UINT64_MAX / x ? overflow() : vb_ok(vb_uint(x * y));
  }
  int64_t x = vb_as_int(a), y = vb_as_int(b);
  return mul_overflows(x, y) ? overflow() : vb_ok(vb_int(x * y));
}
VbVal vb_div_checked(VbVal a, VbVal b) {
  if (either_float(a, b)) {
    double y = vb_as_float(b);
    if (y == 0.0) return fault(&vb_info_DivZero, 1);
    return vb_ok(vb_float(vb_as_float(a) / y));
  }
  int64_t y = vb_as_int(b);
  if (y == 0) return fault(&vb_info_DivZero, 1);
  if (either_unsigned(a, b)) return vb_ok(vb_uint(vb_as_uint(a) / (uint64_t)y));
  int64_t x = vb_as_int(a);
  if (x == INT64_MIN && y == -1) return overflow();
  return vb_ok(vb_int(x / y));
}
VbVal vb_get_checked(VbVal v, VbVal i) {
  VbVec *s = vb_as_vec(v);
  uint64_t k = vb_as_uint(i);
  if (k >= s->n) return fault(&vb_info_OutOfBounds, 2);
  /* Borrows, so the element comes back as a copy, as `vb_lookup`'s does. */
  return vb_ok(vb_dup(s->a[k]));
}
