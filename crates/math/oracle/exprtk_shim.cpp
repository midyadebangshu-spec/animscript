#define exprtk_disable_comments
#define exprtk_disable_string_capabilities
#define exprtk_disable_superscalar_unroll
#define exprtk_disable_rtl_io_file
#define exprtk_disable_rtl_vecops
#define exprtk_disable_caseinsensitivity
#include "exprtk.hpp"
#include <cstdio>
extern "C" double ox_eval(const char* src, const char** names, const double* vals, int n, int* ok) {
    exprtk::symbol_table<double> st;
    std::vector<double> v(vals, vals + n);
    for (int i = 0; i < n; i++) st.add_variable(names[i], v[i]);
    st.add_constants();
    exprtk::expression<double> e; e.register_symbol_table(st);
    exprtk::parser<double> p;
    if (!p.compile(src, e)) { *ok = 0; return 0.0; }
    *ok = 1; return e.value();
}
