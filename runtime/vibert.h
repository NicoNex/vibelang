/* Vibelang bootstrap runtime.
 *
 * One uniform boxed value, calloc/free, and the primitives the prelude needs.
 * Every value is freed where its owner dies, at a point the compiler computed.
 * No GC, no refcount, no signal handler, nothing beyond libc.
 *
 * vibec debt: values are dynamically tagged and arithmetic dispatches on the
 * tag. The type checker already knows every static type, so the upgrade path is
 * to thread resolved types into codegen and emit native C operators. Do that
 * when a benchmark asks for it, not before. */
#ifndef VIBERT_H
#define VIBERT_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

typedef enum {
  VB_UNIT = 0,
  VB_INT,   /* signed machine integer */
  VB_UINT,  /* unsigned machine integer */
  VB_FLOAT,
  VB_BOOL,
  VB_CHAR,
  VB_STR,
  VB_CSTR,
  VB_VEC,
  VB_OBJ,
  VB_CLOS,
  VB_PTR
} VbTag;

typedef struct VbVal {
  uint8_t tag;
  union {
    int64_t i;
    uint64_t u;
    double f;
    bool b;
    uint32_t c;
    void *p;
  } v;
} VbVal;

typedef struct { size_t n; char *p; } VbStr;
typedef struct { size_t n, cap; VbVal *a; } VbVec;

/* Description of a constructor or record, emitted as static data by vibec. */
typedef struct {
  const char *name;
  uint32_t nfields;
  const char *const *fields; /* NULL for variants without field names */
} VbInfo;

typedef struct { const VbInfo *info; uint32_t tag; uint32_t n; VbVal *f; } VbObj;

typedef VbVal (*VbFn)(VbVal *args);
typedef struct { VbFn fn; const char *name; uint32_t arity, nargs; VbVal *args; } VbClos;

/* --- lifecycle --- */
void vb_init(void);
void *vb_alloc(size_t n);
/* Frees one object. */
void vb_free(void *p);
/* Frees one value where its owner dies. Deep for a vector and an object; see
   the note in vibert.c for the two that are not. */
void vb_dispose(VbVal v);

/* --- constructors --- */
VbVal vb_unit(void);
VbVal vb_int(int64_t x);
VbVal vb_uint(uint64_t x);
VbVal vb_float(double x);
VbVal vb_bool(bool x);
VbVal vb_char(uint32_t x);
VbVal vb_ptr(void *p);
VbVal vb_cstr_val(const char *s);
VbVal vb_str(const char *s, size_t n);
VbVal vb_strz(const char *s);
VbVal vb_obj(const VbInfo *info, uint32_t tag, uint32_t n, ...);
VbVal vb_clos(VbFn fn, const char *name, uint32_t arity);

/* --- unboxing, for the C boundary --- */
int64_t vb_as_int(VbVal v);
uint64_t vb_as_uint(VbVal v);
double vb_as_float(VbVal v);
bool vb_as_bool(VbVal v);
const char *vb_as_cstr(VbVal v);
void *vb_as_ptr(VbVal v);
VbStr *vb_as_str(VbVal v);
VbVec *vb_as_vec(VbVal v);
VbObj *vb_as_obj(VbVal v);

/* --- obligations that the bootstrap discharges at run time (spec §7.2) --- */
void vb_require(bool cond, const char *path, const char *what);
void vb_fail(const char *path, const char *what);

/* --- application --- */
VbVal vb_apply1(VbVal f, VbVal x);
VbVal vb_applyn(VbVal f, uint32_t n, VbVal *xs);

/* --- arithmetic / comparison --- */
VbVal vb_add(VbVal a, VbVal b);
VbVal vb_sub(VbVal a, VbVal b);
VbVal vb_mul(VbVal a, VbVal b);
VbVal vb_div(VbVal a, VbVal b, const char *path);
VbVal vb_neg(VbVal a);
bool vb_eq(VbVal a, VbVal b);
int vb_cmp(VbVal a, VbVal b);

/* --- field / variant access --- */
VbVal vb_field(VbVal o, uint32_t i);
uint32_t vb_tag(VbVal o);

VbVal vb_set_fields(VbVal o, uint32_t nchanged, const uint32_t *idx, const VbVal *vals);
VbVal vb_with(VbVal o, uint32_t nchanged, const uint32_t *idx, const VbVal *vals);

/* --- Dict: an association vector, see the note in vibert.c --- */
VbVal vb_dict(void);
VbVal vb_insert(VbVal d, VbVal k, VbVal v);
VbVal vb_lookup(VbVal d, VbVal k);
VbVal vb_remove(VbVal d, VbVal k);
VbVal vb_keys(VbVal d);

/* --- Vec --- */
VbVal vb_vec_new(void);
VbVal vb_vec_lit(uint32_t n, ...);
VbVal vb_push(VbVal v, VbVal x);
VbVal vb_get(VbVal v, VbVal i, const char *path);
VbVal vb_set(VbVal v, VbVal i, VbVal x, const char *path);
VbVal vb_len(VbVal v);
VbVal vb_map(VbVal f, VbVal v);
VbVal vb_filter(VbVal f, VbVal v);
VbVal vb_fold(VbVal f, VbVal z, VbVal v);
VbVal vb_each(VbVal f, VbVal v);
VbVal vb_sum(VbVal v);
VbVal vb_max_by(VbVal f, VbVal v, const char *path);
VbVal vb_min_by(VbVal f, VbVal v, const char *path);
VbVal vb_sort_by(VbVal f, VbVal v);
VbVal vb_rev(VbVal v);
VbVal vb_concat_vec(VbVal a, VbVal b);
VbVal vb_range(VbVal a, VbVal b);
VbVal vb_single(VbVal x);
VbVal vb_take(VbVal n, VbVal v);
VbVal vb_drop(VbVal n, VbVal v);

/* --- Str --- */
VbVal vb_split(VbVal c, VbVal s);
VbVal vb_lines(VbVal s);
VbVal vb_dup(VbVal v);
VbVal vb_concat(VbVal a, VbVal b);
VbVal vb_trim(VbVal s);
VbVal vb_starts_with(VbVal s, VbVal p);
VbVal vb_contains(VbVal s, VbVal p);
VbVal vb_to_cstr(VbVal s);
VbVal vb_from_cstr(VbVal p);
VbVal vb_chr(VbVal c);
VbVal vb_slice(VbVal i, VbVal j, VbVal s);
VbVal vb_index_of(VbVal s, VbVal p);
VbVal vb_replace(VbVal s, VbVal from, VbVal to);
VbVal vb_lower(VbVal s);
VbVal vb_show(VbVal v);
VbVal vb_fmt(VbVal f, uint32_t n, ...);

/* --- conversions --- */
VbVal vb_to_f64(VbVal v);
VbVal vb_to_f32(VbVal v);
VbVal vb_to_signed(VbVal v, int bits);
VbVal vb_to_unsigned(VbVal v, int bits);

/* --- Res / Opt, shared with generated code --- */
extern const VbInfo vb_info_Ok, vb_info_Er, vb_info_Some, vb_info_None;
VbVal vb_ok(VbVal x);
VbVal vb_er(VbVal x);
VbVal vb_some(VbVal x);
VbVal vb_none(void);
VbVal vb_seq(VbVal v);
VbVal vb_parse_int(VbVal s, int sign);
VbVal vb_parse_f64(VbVal s);

/* --- Math --- */
VbVal vb_band(VbVal a, VbVal b);
VbVal vb_bor(VbVal a, VbVal b);
VbVal vb_bxor(VbVal a, VbVal b);
VbVal vb_bnot(VbVal a);
VbVal vb_shl(VbVal a, VbVal n);
VbVal vb_shr(VbVal a, VbVal n);
VbVal vb_ord(VbVal c);
VbVal vb_abs(VbVal a);
VbVal vb_min(VbVal a, VbVal b);
VbVal vb_max(VbVal a, VbVal b);
VbVal vb_sqrt(VbVal a);
VbVal vb_pow(VbVal a, VbVal b);
VbVal vb_floor(VbVal a);
VbVal vb_show_exact(VbVal a);
VbVal vb_byte_at(VbVal s, VbVal i, const char *path);

/* --- IO --- */
VbVal vb_read(VbVal path);
VbVal vb_read_stdin(void);
VbVal vb_write(VbVal path, VbVal data);
VbVal vb_out(VbVal s);
VbVal vb_warn(VbVal s);
VbVal vb_argv(void);
VbVal vb_exit(VbVal code);
void vb_set_args(int argc, char **argv);

/* --- Checked (spec §7.5) --- */
VbVal vb_add_checked(VbVal a, VbVal b);
VbVal vb_sub_checked(VbVal a, VbVal b);
VbVal vb_mul_checked(VbVal a, VbVal b);
VbVal vb_div_checked(VbVal a, VbVal b);
VbVal vb_get_checked(VbVal v, VbVal i);
extern const VbInfo vb_info_Overflow, vb_info_DivZero, vb_info_OutOfBounds, vb_info_BadParse;

#endif /* VIBERT_H */
