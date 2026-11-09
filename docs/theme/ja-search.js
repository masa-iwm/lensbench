// 日本語の検索（mdBook の索引は語を空白で区切るので、日本語の語では当たらない）。
// 日本語を含む検索語は、索引の各節の本文・見出しを部分一致で探す。空白で区切った語はすべて含む節だけ。
(function () {
    if (typeof elasticlunr === 'undefined') return;
    const cjk = /[　-ヿ㐀-鿿豈-﫿＀-￯]/;
    const orig = elasticlunr.Index.prototype.search;
    function count(s, t) {
        let n = 0;
        for (let i = s.indexOf(t); i >= 0; i = s.indexOf(t, i + t.length)) n++;
        return n;
    }
    elasticlunr.Index.prototype.search = function (query, config) {
        if (!cjk.test(query)) return orig.call(this, query, config);
        const terms = query.toLowerCase().split(/\s+/).filter(Boolean);
        const docs = this.documentStore.docs;
        const out = [];
        for (const ref in docs) {
            const d = docs[ref];
            const head = ((d.title || '') + ' ' + (d.breadcrumbs || '')).toLowerCase();
            const body = (d.body || '').toLowerCase();
            let score = 0;
            let ok = true;
            for (const t of terms) {
                const inHead = head.includes(t);
                const c = count(body, t);
                if (!inHead && c === 0) { ok = false; break; }
                score += c + (inHead ? 10 : 0);
            }
            if (ok) out.push({ ref: ref, score: score, doc: d });
        }
        out.sort(function (a, b) { return b.score - a.score; });
        return out;
    };
})();
