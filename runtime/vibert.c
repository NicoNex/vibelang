#include "vibert.h"

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

void *vb_alloc(size_t n) {
  void *p = calloc(1, n ? n : 1);
  if (!p) { fputs("vibe: out of memory\n", stderr); exit(70); }
  return p;
}

void vb_free(void *p) { free(p); }

/* ---------------------------------------------------------- constructors */

VbVal vb_unit(void) { VbVal v; v.tag = VB_UNIT; v.v.i = 0; return v; }
VbVal vb_int(int64_t x) { VbVal v; v.tag = VB_INT; v.v.i = x; return v; }
VbVal vb_uint(uint64_t x) { VbVal v; v.tag = VB_UINT; v.v.u = x; return v; }
VbVal vb_float(double x) { VbVal v; v.tag = VB_FLOAT; v.v.f = x; return v; }
VbVal vb_bool(bool x) { VbVal v; v.tag = VB_BOOL; v.v.b = x; return v; }
VbVal vb_char(uint32_t x) { VbVal v; v.tag = VB_CHAR; v.v.c = x; return v; }
VbVal vb_ptr(void *p) { VbVal v; v.tag = VB_PTR; v.v.p = p; return v; }
VbVal vb_cstr_val(const char *s) { VbVal v; v.tag = VB_CSTR; v.v.p = (void *)s; return v; }

VbVal vb_str(const char *s, size_t n) {
  VbStr *o = vb_alloc(sizeof(VbStr));
  o->n = n;
  o->p = vb_alloc(n + 1);
  if (n) memcpy(o->p, s, n);
  o->p[n] = 0;
  VbVal v; v.tag = VB_STR; v.v.p = o; return v;
}
/* `vb_str` copies, so a buffer built only to be copied is freed here. */
static VbVal str_take(char *buf, size_t n) {
  VbVal r = vb_str(buf, n);
  free(buf);
  return r;
}
VbVal vb_strz(const char *s) { return vb_str(s ? s : "", s ? strlen(s) : 0); }

VbVal vb_obj(const VbInfo *info, uint32_t tag, uint32_t n, ...) {
  VbObj *o = vb_alloc(sizeof(VbObj));
  o->info = info; o->tag = tag; o->n = n;
  o->f = n ? vb_alloc(sizeof(VbVal) * n) : NULL;
  va_list ap; va_start(ap, n);
  for (uint32_t i = 0; i < n; i++) o->f[i] = va_arg(ap, VbVal);
  va_end(ap);
  VbVal v; v.tag = VB_OBJ; v.v.p = o; return v;
}

VbVal vb_clos(VbFn fn, const char *name, uint32_t arity) {
  VbClos *c = vb_alloc(sizeof(VbClos));
  c->fn = fn; c->name = name; c->arity = arity; c->nargs = 0;
  c->args = arity ? vb_alloc(sizeof(VbVal) * arity) : NULL;
  VbVal v; v.tag = VB_CLOS; v.v.p = c; return v;
}

/* Free one value, at the point its owner dies (spec 4.2,
   docs/static-drop-roadmap.md).

   Deep for a vector and for an object, because nothing else points at what
   they hold. Three things had to be true first, and now are: the operations
   that take `&Vec` copy each element rather than its pointer (see `vb_push`
   above); a function may no longer return a piece of a borrowed parameter
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
    fputs("  this obligation is checked at run time in the bootstrap; fase 6 discharges it statically\n",
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
    case VB_FLOAT: return (int64_t)v.v.f;
    case VB_BOOL: return v.v.b ? 1 : 0;
    case VB_CHAR: return (int64_t)v.v.c;
    default: vb_fail("<runtime>", "expected an integer"); return 0;
  }
}
uint64_t vb_as_uint(VbVal v) { return (uint64_t)vb_as_int(v); }
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
  VbClos *n = vb_alloc(sizeof(VbClos));
  *n = *c;
  n->args = vb_alloc(sizeof(VbVal) * (c->arity ? c->arity : 1));
  for (uint32_t i = 0; i < c->nargs; i++) n->args[i] = c->args[i];
  n->args[n->nargs++] = x;
  if (n->nargs == n->arity) {
    /* The saturated copy exists for the duration of the call and nothing can
       have kept it: the body reads its arguments out of the array, and a
       closure it builds allocates an array of its own.
       ponytail: the intermediate copies of a partial application are not freed
       here, because the last one is still live when this returns. They die with
       the value that holds them, which is the caller's drop to place. */
    VbVal r = n->fn(n->args);
    vb_free(n->args);
    vb_free(n);
    return r;
  }
  VbVal v; v.tag = VB_CLOS; v.v.p = n; return v;
}


/* ------------------------------------------------------------ arithmetic */

static bool either_float(VbVal a, VbVal b) { return a.tag == VB_FLOAT || b.tag == VB_FLOAT; }
static bool either_unsigned(VbVal a, VbVal b) { return a.tag == VB_UINT || b.tag == VB_UINT; }

VbVal vb_add(VbVal a, VbVal b) {
  if (a.tag == VB_STR && b.tag == VB_STR) return vb_concat(a, b);
  if (either_float(a, b)) return vb_float(vb_as_float(a) + vb_as_float(b));
  if (either_unsigned(a, b)) return vb_uint(vb_as_uint(a) + vb_as_uint(b));
  return vb_int(vb_as_int(a) + vb_as_int(b));
}
VbVal vb_sub(VbVal a, VbVal b) {
  if (either_float(a, b)) return vb_float(vb_as_float(a) - vb_as_float(b));
  if (either_unsigned(a, b)) {
    uint64_t x = vb_as_uint(a), y = vb_as_uint(b);
    vb_require(x >= y, "<program>", "unsigned subtraction would wrap");
    return vb_uint(x - y);
  }
  return vb_int(vb_as_int(a) - vb_as_int(b));
}
VbVal vb_mul(VbVal a, VbVal b) {
  if (either_float(a, b)) return vb_float(vb_as_float(a) * vb_as_float(b));
  if (either_unsigned(a, b)) return vb_uint(vb_as_uint(a) * vb_as_uint(b));
  return vb_int(vb_as_int(a) * vb_as_int(b));
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
  return vb_int(vb_as_int(a) / y);
}
VbVal vb_neg(VbVal a) {
  if (a.tag == VB_FLOAT) return vb_float(-a.v.f);
  return vb_int(-vb_as_int(a));
}

int vb_cmp(VbVal a, VbVal b) {
  if (a.tag == VB_STR && b.tag == VB_STR) {
    VbStr *x = vb_as_str(a), *y = vb_as_str(b);
    size_t n = x->n < y->n ? x->n : y->n;
    int c = memcmp(x->p, y->p, n);
    if (c) return c < 0 ? -1 : 1;
    return x->n == y->n ? 0 : (x->n < y->n ? -1 : 1);
  }
  if (either_float(a, b)) {
    double x = vb_as_float(a), y = vb_as_float(b);
    return x < y ? -1 : (x > y ? 1 : 0);
  }
  if (either_unsigned(a, b)) {
    uint64_t x = vb_as_uint(a), y = vb_as_uint(b);
    return x < y ? -1 : (x > y ? 1 : 0);
  }
  int64_t x = vb_as_int(a), y = vb_as_int(b);
  return x < y ? -1 : (x > y ? 1 : 0);
}

bool vb_eq(VbVal a, VbVal b) {
  if (a.tag == VB_UNIT && b.tag == VB_UNIT) return true;
  if (a.tag == VB_BOOL && b.tag == VB_BOOL) return a.v.b == b.v.b;
  if (a.tag == VB_CHAR && b.tag == VB_CHAR) return a.v.c == b.v.c;
  if (a.tag == VB_OBJ && b.tag == VB_OBJ) {
    VbObj *x = vb_as_obj(a), *y = vb_as_obj(b);
    if (x->tag != y->tag || x->n != y->n) return false;
    for (uint32_t i = 0; i < x->n; i++)
      if (!vb_eq(x->f[i], y->f[i])) return false;
    return true;
  }
  if (a.tag == VB_VEC && b.tag == VB_VEC) {
    VbVec *x = vb_as_vec(a), *y = vb_as_vec(b);
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
  VbObj *y = vb_alloc(sizeof(VbObj));
  y->info = x->info; y->tag = x->tag; y->n = x->n;
  y->f = x->n ? vb_alloc(sizeof(VbVal) * x->n) : NULL;
  for (uint32_t i = 0; i < x->n; i++) y->f[i] = x->f[i];
  for (uint32_t i = 0; i < nchanged; i++) y->f[idx[i]] = vals[i];
  VbVal v; v.tag = VB_OBJ; v.v.p = y; return v;
}

/* ---------------------------------------------------------------- Vec */

static VbVal wrap_vec(VbVec *w) { VbVal v; v.tag = VB_VEC; v.v.p = w; return v; }

static VbVec *vec_alloc(size_t cap) {
  VbVec *w = vb_alloc(sizeof(VbVec));
  w->n = 0; w->cap = cap;
  w->a = cap ? vb_alloc(sizeof(VbVal) * cap) : NULL;
  return w;
}
static void vec_push(VbVec *w, VbVal x) {
  if (w->n == w->cap) {
    size_t cap = w->cap ? w->cap * 2 : 8;
    /* The array is internal to this vector: no other value points at it. */
    VbVal *a = realloc(w->a, sizeof(VbVal) * cap);
    if (!a) { fputs("vibe: out of memory\n", stderr); exit(70); }
    w->a = a; w->cap = cap;
  }
  w->a[w->n++] = x;
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

/* `push` and `set` take the vector by value, so the old spine is already the
 * caller's to lose and its elements move rather than copy. The operations that
 * take `&Vec` — `rev`, `filter`, `sort_by`, `take`, `drop`, `concat_vec` —
 * cannot do that: the caller still owns the source and will free it, so each
 * element is copied with `vb_dup` and no two vectors point at one value
 * (docs/aliasing-audit.md, gap 3). That copy is what makes `vb_dispose` able to
 * be deep.
 *
 * ponytail: the copy is unconditional. It is dead work when the source dies at
 * the call, and the drop table already computes last use — eliding it there is
 * Open decision 5 of docs/static-drop-roadmap.md, and it needs a measurement
 * first. */
VbVal vb_push(VbVal v, VbVal x) {
  VbVec *s = vb_as_vec(v);
  VbVec *w = vec_alloc(s->n + 1);
  for (size_t i = 0; i < s->n; i++) vec_push(w, s->a[i]);
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
  uint64_t k = vb_as_uint(i);
  vb_require(k < s->n, path, "i < len xs");
  s->a[k] = x;
  return v;
}

VbVal vb_byte_at(VbVal s, VbVal i, const char *path) {
  VbStr *x = vb_as_str(s);
  uint64_t k = vb_as_uint(i);
  vb_require(k < x->n, path, "i < len s");
  return vb_uint((unsigned char)x->p[k]);
}
VbVal vb_get(VbVal v, VbVal i, const char *path) {
  VbVec *s = vb_as_vec(v);
  uint64_t k = vb_as_uint(i);
  vb_require(k < s->n, path, "i < len xs");
  return s->a[k];
}
VbVal vb_set(VbVal v, VbVal i, VbVal x, const char *path) {
  VbVec *s = vb_as_vec(v);
  uint64_t k = vb_as_uint(i);
  vb_require(k < s->n, path, "i < len xs");
  VbVec *w = vec_alloc(s->n);
  for (size_t j = 0; j < s->n; j++) vec_push(w, s->a[j]);
  w->a[k] = x;
  return wrap_vec(w);
}
VbVal vb_len(VbVal v) {
  if (v.tag == VB_STR) return vb_uint(vb_as_str(v)->n);
  return vb_uint(vb_as_vec(v)->n);
}
VbVal vb_map(VbVal f, VbVal v) {
  VbVec *s = vb_as_vec(v);
  VbVec *w = vec_alloc(s->n);
  for (size_t i = 0; i < s->n; i++) vec_push(w, vb_apply1(f, s->a[i]));
  return wrap_vec(w);
}
VbVal vb_filter(VbVal f, VbVal v) {
  VbVec *s = vb_as_vec(v);
  VbVec *w = vec_alloc(s->n);
  for (size_t i = 0; i < s->n; i++)
    if (vb_as_bool(vb_apply1(f, s->a[i]))) vec_push(w, vb_dup(s->a[i]));
  return wrap_vec(w);
}
VbVal vb_fold(VbVal f, VbVal z, VbVal v) {
  VbVec *s = vb_as_vec(v);
  for (size_t i = 0; i < s->n; i++) z = vb_apply1(vb_apply1(f, z), s->a[i]);
  return z;
}
VbVal vb_each(VbVal f, VbVal v) {
  VbVec *s = vb_as_vec(v);
  for (size_t i = 0; i < s->n; i++) vb_apply1(f, s->a[i]);
  return vb_unit();
}
VbVal vb_sum(VbVal v) {
  VbVec *s = vb_as_vec(v);
  if (s->n == 0) return vb_int(0);
  VbVal acc = s->a[0];
  for (size_t i = 1; i < s->n; i++) acc = vb_add(acc, s->a[i]);
  return acc;
}
/* The first element whose key compares strictly beyond every earlier one in
   the direction of `sign`: +1 for the largest, -1 for the smallest. */
static VbVal extreme_by(VbVal f, VbVal v, const char *path, int sign) {
  VbVec *s = vb_as_vec(v);
  vb_require(s->n > 0, path, "len xs > 0");
  size_t best = 0; VbVal bk = vb_apply1(f, s->a[0]);
  for (size_t i = 1; i < s->n; i++) {
    VbVal k = vb_apply1(f, s->a[i]);
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
  KeyVal *kv = vb_alloc(sizeof(KeyVal) * (s->n ? s->n : 1));
  KeyVal *tmp = vb_alloc(sizeof(KeyVal) * (s->n ? s->n : 1));
  for (size_t i = 0; i < s->n; i++) { kv[i].val = s->a[i]; kv[i].key = vb_apply1(f, s->a[i]); }
  merge_sort(kv, tmp, s->n);
  VbVec *w = vec_alloc(s->n);
  for (size_t i = 0; i < s->n; i++) vec_push(w, vb_dup(kv[i].val));
  free(kv);
  free(tmp);
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
  for (size_t i = 0; i < x->n; i++) vec_push(w, vb_dup(x->a[i]));
  for (size_t i = 0; i < y->n; i++) vec_push(w, vb_dup(y->a[i]));
  return wrap_vec(w);
}
VbVal vb_take(VbVal n, VbVal v) {
  VbVec *s = vb_as_vec(v);
  uint64_t k = vb_as_uint(n);
  if (k > s->n) k = s->n;
  VbVec *w = vec_alloc((size_t)k);
  for (size_t i = 0; i < (size_t)k; i++) vec_push(w, vb_dup(s->a[i]));
  return wrap_vec(w);
}
VbVal vb_drop(VbVal n, VbVal v) {
  VbVec *s = vb_as_vec(v);
  uint64_t k = vb_as_uint(n);
  if (k > s->n) k = s->n;
  VbVec *w = vec_alloc(s->n - (size_t)k);
  for (size_t i = (size_t)k; i < s->n; i++) vec_push(w, vb_dup(s->a[i]));
  return wrap_vec(w);
}
VbVal vb_range(VbVal a, VbVal b) {
  uint64_t lo = vb_as_uint(a), hi = vb_as_uint(b);
  VbVec *w = vec_alloc(hi > lo ? hi - lo : 0);
  for (uint64_t i = lo; i < hi; i++) vec_push(w, vb_uint(i));
  return wrap_vec(w);
}

/* ---------------------------------------------------------------- Str */

VbVal vb_split(VbVal c, VbVal s) {
  VbStr *x = vb_as_str(s);
  char sep = (char)vb_as_int(c);
  VbVec *w = vec_alloc(4);
  size_t start = 0;
  for (size_t i = 0; i <= x->n; i++) {
    if (i == x->n || x->p[i] == sep) { vec_push(w, vb_str(x->p + start, i - start)); start = i + 1; }
  }
  return wrap_vec(w);
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
  if (start < x->n) vec_push(w, vb_str(x->p + start, x->n - start));
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
      for (size_t i = 0; i < x->n; i++) vec_push(w, vb_dup(x->a[i]));
      return wrap_vec(w);
    }
    case VB_OBJ: {
      VbObj *x = v.v.p;
      VbObj *o = vb_alloc(sizeof(VbObj));
      o->info = x->info; o->tag = x->tag; o->n = x->n;
      o->f = x->n ? vb_alloc(sizeof(VbVal) * x->n) : NULL;
      for (uint32_t i = 0; i < x->n; i++) o->f[i] = vb_dup(x->f[i]);
      VbVal r; r.tag = VB_OBJ; r.v.p = o; return r;
    }
    case VB_CLOS: {
      VbClos *x = v.v.p;
      VbClos *c = vb_alloc(sizeof(VbClos));
      c->fn = x->fn; c->name = x->name; c->arity = x->arity; c->nargs = x->nargs;
      c->args = x->arity ? vb_alloc(sizeof(VbVal) * x->arity) : NULL;
      for (uint32_t i = 0; i < x->nargs; i++) c->args[i] = vb_dup(x->args[i]);
      VbVal r; r.tag = VB_CLOS; r.v.p = c; return r;
    }
    /* A scalar is already a copy; a C pointer is not ours to duplicate. */
    default: return v;
  }
}
VbVal vb_concat(VbVal a, VbVal b) {
  VbStr *x = vb_as_str(a), *y = vb_as_str(b);
  VbStr *o = vb_alloc(sizeof(VbStr));
  o->n = x->n + y->n;
  o->p = vb_alloc(o->n + 1);
  memcpy(o->p, x->p, x->n);
  memcpy(o->p + x->n, y->p, y->n);
  o->p[o->n] = 0;
  VbVal v; v.tag = VB_STR; v.v.p = o; return v;
}
VbVal vb_trim(VbVal s) {
  VbStr *x = vb_as_str(s);
  size_t i = 0, j = x->n;
  while (i < j && (x->p[i] == ' ' || x->p[i] == '\t' || x->p[i] == '\n' || x->p[i] == '\r')) i++;
  while (j > i && (x->p[j-1] == ' ' || x->p[j-1] == '\t' || x->p[j-1] == '\n' || x->p[j-1] == '\r')) j--;
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
VbVal vb_contains(VbVal s, VbVal p) {
  return vb_bool(str_find(vb_as_str(s), vb_as_str(p), 0) != SIZE_MAX);
}
VbVal vb_to_cstr(VbVal s) { return vb_cstr_val(vb_as_str(s)->p); }
VbVal vb_from_cstr(VbVal p) { return vb_strz((const char *)vb_as_ptr(p)); }
VbVal vb_chr(VbVal c) { char b = (char)vb_as_int(c); return vb_str(&b, 1); }
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
/* Non-overlapping, left to right. An empty needle replaces nothing. */
VbVal vb_replace(VbVal s, VbVal from, VbVal to) {
  VbStr *x = vb_as_str(s), *f = vb_as_str(from), *t = vb_as_str(to);
  if (f->n == 0 || f->n > x->n) return vb_str(x->p, x->n);
  size_t hits = 0;
  for (size_t i = str_find(x, f, 0); i != SIZE_MAX; i = str_find(x, f, i + f->n)) hits++;
  if (hits == 0) return vb_str(x->p, x->n);
  size_t n = x->n - hits * f->n + hits * t->n;
  char *buf = vb_alloc(n + 1);
  size_t w = 0, done = 0; /* `done`: the input already copied */
  for (size_t i = str_find(x, f, 0); i != SIZE_MAX; i = str_find(x, f, done)) {
    memcpy(buf + w, x->p + done, i - done); w += i - done;
    memcpy(buf + w, t->p, t->n); w += t->n;
    done = i + f->n;
  }
  memcpy(buf + w, x->p + done, x->n - done);
  return str_take(buf, n);
}
/* ponytail: ASCII only. Upgrade path is a UTF-8 case table, when a program that
   needs one exists. */
VbVal vb_lower(VbVal s) {
  VbStr *x = vb_as_str(s);
  char *buf = vb_alloc(x->n + 1);
  for (size_t i = 0; i < x->n; i++) {
    char c = x->p[i];
    buf[i] = (c >= 'A' && c <= 'Z') ? (char)(c - 'A' + 'a') : c;
  }
  return str_take(buf, x->n);
}

/* A growable byte buffer for `show` and `fmt`. */
typedef struct { char *p; size_t n, cap; } Sb;
static void sb_push(Sb *acc, const char *s, size_t n) {
  if (acc->n + n > acc->cap) {
    size_t cap = acc->cap ? acc->cap : 64;
    while (cap < acc->n + n) cap *= 2;
    char *p = realloc(acc->p, cap);
    if (!p) { fputs("vibe: out of memory\n", stderr); exit(70); }
    acc->p = p; acc->cap = cap;
  }
  if (n) memcpy(acc->p + acc->n, s, n);
  acc->n += n;
}
static VbVal sb_done(Sb *acc) {
  VbVal r = vb_str(acc->p ? acc->p : "", acc->n);
  free(acc->p);
  return r;
}

static void show_into(Sb *acc, VbVal v) {
  char tmp[64];
  switch (v.tag) {
    case VB_UNIT: sb_push(acc, "()", 2); break;
    case VB_INT: sb_push(acc, tmp, (size_t)snprintf(tmp, sizeof tmp, "%lld", (long long)v.v.i)); break;
    case VB_UINT: sb_push(acc, tmp, (size_t)snprintf(tmp, sizeof tmp, "%llu", (unsigned long long)v.v.u)); break;
    case VB_FLOAT: sb_push(acc, tmp, (size_t)snprintf(tmp, sizeof tmp, "%g", v.v.f)); break;
    case VB_BOOL: sb_push(acc, v.v.b ? "True" : "False", v.v.b ? 4 : 5); break;
    case VB_CHAR: { char c = (char)v.v.c; sb_push(acc, &c, 1); break; }
    case VB_STR: { VbStr *s = (VbStr *)v.v.p; sb_push(acc, s->p, s->n); break; }
    case VB_CSTR: { const char *s = (const char *)v.v.p; sb_push(acc, s ? s : "", s ? strlen(s) : 0); break; }
    case VB_PTR: sb_push(acc, tmp, (size_t)snprintf(tmp, sizeof tmp, "0x%llx", (unsigned long long)(uintptr_t)v.v.p)); break;
    case VB_CLOS: { VbClos *c = (VbClos *)v.v.p; sb_push(acc, "<", 1); sb_push(acc, c->name, strlen(c->name)); sb_push(acc, ">", 1); break; }
    case VB_VEC: {
      VbVec *s = (VbVec *)v.v.p;
      sb_push(acc, "[", 1);
      for (size_t i = 0; i < s->n; i++) { if (i) sb_push(acc, ", ", 2); show_into(acc, s->a[i]); }
      sb_push(acc, "]", 1);
      break;
    }
    case VB_OBJ: {
      VbObj *o = (VbObj *)v.v.p;
      const char *name = o->info ? o->info->name : "?";
      if (o->info && o->info->fields) {
        sb_push(acc, "{", 1);
        for (uint32_t i = 0; i < o->n; i++) {
          if (i) sb_push(acc, ", ", 2);
          sb_push(acc, o->info->fields[i], strlen(o->info->fields[i]));
          sb_push(acc, "=", 1);
          show_into(acc, o->f[i]);
        }
        sb_push(acc, "}", 1);
      } else {
        sb_push(acc, name, strlen(name));
        for (uint32_t i = 0; i < o->n; i++) { sb_push(acc, " ", 1); show_into(acc, o->f[i]); }
      }
      break;
    }
    default: sb_push(acc, "?", 1);
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
      if (k < n) { show_into(&acc, va_arg(ap, VbVal)); k++; } else sb_push(&acc, "{}", 2);
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
  uint64_t x = v.tag == VB_FLOAT ? (uint64_t)v.v.f : (uint64_t)vb_as_int(v);
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
const VbInfo vb_info_BadParse = {"BadParse", 0, NULL};

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

/* Takes the dictionary by value, so the entries move rather than copy and the
   old spine is this function's to free. An entry the new key displaces is
   nobody's after this, so it goes too. */
VbVal vb_insert(VbVal d, VbVal k, VbVal v) {
  VbVec *s = vb_as_vec(d);
  size_t at = dict_find(s, k);
  VbVec *w = vec_alloc(s->n + 1);
  for (size_t i = 0; i < s->n; i++) {
    if (i == at) vb_dispose(s->a[i]); else vec_push(w, s->a[i]);
  }
  vec_push(w, vb_obj(&vb_info_entry, 0, 2, k, v));
  vb_free(s->a);
  vb_free(s);
  return wrap_vec(w);
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

VbVal vb_remove(VbVal d, VbVal k) {
  VbVec *s = vb_as_vec(d);
  size_t at = dict_find(s, k);
  VbVec *w = vec_alloc(s->n);
  for (size_t i = 0; i < s->n; i++) {
    if (i == at) vb_dispose(s->a[i]); else vec_push(w, s->a[i]);
  }
  vb_free(s->a);
  vb_free(s);
  return wrap_vec(w);
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
    if (o->tag != 0) return s->a[i]; /* the first Er wins */
    vec_push(w, o->f[0]);
  }
  return vb_ok(wrap_vec(w));
}

VbVal vb_parse_int(VbVal s, int sign) {
  VbStr *x = vb_as_str(s);
  if (x->n == 0) return vb_none();
  char *end = NULL;
  if (sign) {
    long long r = strtoll(x->p, &end, 10);
    if (end != x->p + x->n) return vb_none();
    return vb_some(vb_int((int64_t)r));
  }
  if (x->p[0] == '-') return vb_none();
  unsigned long long r = strtoull(x->p, &end, 10);
  if (end != x->p + x->n) return vb_none();
  return vb_some(vb_uint((uint64_t)r));
}
VbVal vb_parse_f64(VbVal s) {
  VbStr *x = vb_as_str(s);
  if (x->n == 0) return vb_none();
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
  return vb_int(x < 0 ? -x : x);
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
  return (a.tag == VB_UINT || b.tag == VB_UINT) ? vb_uint(x) : vb_int((int64_t)x);
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
  uint64_t x = ~bits(a, "bnot needs an integer");
  return a.tag == VB_UINT ? vb_uint(x) : vb_int((int64_t)x);
}

/* A shift wider than the word is zero — or, for a signed right shift of a
   negative value, all ones. C leaves both undefined, which is not an answer a
   generated program can be given. */
VbVal vb_shl(VbVal a, VbVal n) {
  uint64_t k = bits(n, "a shift count is an integer");
  uint64_t x = bits(a, "shl needs an integer");
  uint64_t r = k >= 64 ? 0 : x << k;
  return a.tag == VB_UINT ? vb_uint(r) : vb_int((int64_t)r);
}

VbVal vb_shr(VbVal a, VbVal n) {
  uint64_t k = bits(n, "a shift count is an integer");
  if (a.tag == VB_UINT) {
    uint64_t x = a.v.u;
    return vb_uint(k >= 64 ? 0 : x >> k);
  }
  /* Signed: arithmetic, so the sign is kept. */
  int64_t x = vb_as_int(a);
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

VbVal vb_read(VbVal path) {
  const char *p = vb_as_str(path)->p;
  FILE *fh = fopen(p, "rb");
  if (!fh) { fprintf(stderr, "✗ read ⊨ cannot open %s\n", p); exit(66); }
  fseek(fh, 0, SEEK_END);
  long n = ftell(fh);
  fseek(fh, 0, SEEK_SET);
  if (n < 0) n = 0;
  char *buf = vb_alloc((size_t)n + 1);
  size_t got = fread(buf, 1, (size_t)n, fh);
  fclose(fh);
  return str_take(buf, got);
}
/* Reads all of standard input. ponytail: doubling buffer with a copy on each
   growth, so peak memory is about twice the input. Stream it if a program ever has to filter more than it can hold. */
VbVal vb_read_stdin(void) {
  size_t cap = 65536, n = 0;
  char *buf = vb_alloc(cap);
  for (;;) {
    size_t got = fread(buf + n, 1, cap - n, stdin);
    n += got;
    if (n < cap) break; /* short read: end of file, or an error */
    char *bigger = realloc(buf, cap * 2);
    if (!bigger) { fputs("vibe: out of memory\n", stderr); exit(70); }
    buf = bigger;
    cap *= 2;
  }
  return str_take(buf, n);
}
VbVal vb_write(VbVal path, VbVal data) {
  const char *p = vb_as_str(path)->p;
  VbStr *d = vb_as_str(data);
  FILE *fh = fopen(p, "wb");
  if (!fh) { fprintf(stderr, "✗ write ⊨ cannot open %s\n", p); exit(73); }
  fwrite(d->p, 1, d->n, fh);
  fclose(fh);
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

VbVal vb_add_checked(VbVal a, VbVal b) {
  if (either_float(a, b)) return vb_ok(vb_float(vb_as_float(a) + vb_as_float(b)));
  if (either_unsigned(a, b)) {
    uint64_t x = vb_as_uint(a), y = vb_as_uint(b);
    if (x > UINT64_MAX - y) return vb_er(vb_obj(&vb_info_Overflow, 0, 0));
    return vb_ok(vb_uint(x + y));
  }
  int64_t x = vb_as_int(a), y = vb_as_int(b), r;
  if (__builtin_add_overflow(x, y, &r)) return vb_er(vb_obj(&vb_info_Overflow, 0, 0));
  return vb_ok(vb_int(r));
}
VbVal vb_sub_checked(VbVal a, VbVal b) {
  if (either_float(a, b)) return vb_ok(vb_float(vb_as_float(a) - vb_as_float(b)));
  if (either_unsigned(a, b)) {
    uint64_t x = vb_as_uint(a), y = vb_as_uint(b);
    if (x < y) return vb_er(vb_obj(&vb_info_Overflow, 0, 0));
    return vb_ok(vb_uint(x - y));
  }
  int64_t x = vb_as_int(a), y = vb_as_int(b), r;
  if (__builtin_sub_overflow(x, y, &r)) return vb_er(vb_obj(&vb_info_Overflow, 0, 0));
  return vb_ok(vb_int(r));
}
VbVal vb_mul_checked(VbVal a, VbVal b) {
  if (either_float(a, b)) return vb_ok(vb_float(vb_as_float(a) * vb_as_float(b)));
  if (either_unsigned(a, b)) {
    uint64_t x = vb_as_uint(a), y = vb_as_uint(b), r;
    if (__builtin_mul_overflow(x, y, &r)) return vb_er(vb_obj(&vb_info_Overflow, 0, 0));
    return vb_ok(vb_uint(r));
  }
  int64_t x = vb_as_int(a), y = vb_as_int(b), r;
  if (__builtin_mul_overflow(x, y, &r)) return vb_er(vb_obj(&vb_info_Overflow, 0, 0));
  return vb_ok(vb_int(r));
}
VbVal vb_div_checked(VbVal a, VbVal b) {
  if (either_float(a, b)) {
    double y = vb_as_float(b);
    if (y == 0.0) return vb_er(vb_obj(&vb_info_DivZero, 1, 0));
    return vb_ok(vb_float(vb_as_float(a) / y));
  }
  int64_t y = vb_as_int(b);
  if (y == 0) return vb_er(vb_obj(&vb_info_DivZero, 1, 0));
  if (either_unsigned(a, b)) return vb_ok(vb_uint(vb_as_uint(a) / (uint64_t)y));
  return vb_ok(vb_int(vb_as_int(a) / y));
}
VbVal vb_get_checked(VbVal v, VbVal i) {
  VbVec *s = vb_as_vec(v);
  uint64_t k = vb_as_uint(i);
  if (k >= s->n) return vb_er(vb_obj(&vb_info_OutOfBounds, 2, 0));
  return vb_ok(s->a[k]);
}
