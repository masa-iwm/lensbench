// 版の切り替え。サイトの根の versions.json（scripts/docs_versions.py が Pages を組むときに書く）にある版を
// メニューの右に並べ、古い版（/vX.Y/）のページには最新の同じページへの案内を出す。versions.json が無ければ（手元など）何もしない
(function () {
    const m = location.pathname.match(/^(.*?\/)v(\d+\.\d+)\/(.*)$/);
    let root, cur, rest;
    if (m) {
        root = m[1]; cur = m[2]; rest = m[3];
    } else {
        root = new URL(typeof path_to_root === 'string' ? path_to_root : '', location.href).pathname;
        cur = null;
        rest = location.pathname.startsWith(root) ? location.pathname.slice(root.length) : '';
    }
    fetch(root + 'versions.json')
        .then(function (r) { return r.ok ? r.json() : null; })
        .then(function (v) {
            if (!v || !Array.isArray(v.versions)) return;
            const sel = document.createElement('select');
            sel.className = 'lb-version';
            sel.title = '版';
            sel.setAttribute('aria-label', '版');
            sel.style.cssText = 'margin:0 8px;font-size:1.4rem;background:var(--bg);color:var(--fg);border:1px solid var(--theme-popup-border);border-radius:4px';
            const add = function (label, path, on) {
                const o = document.createElement('option');
                o.textContent = label;
                o.value = root + path + rest;
                o.selected = on;
                sel.appendChild(o);
            };
            // main の版がまだリリースしていない版（0.5.0-dev など）なら「開発中」
            add('最新（main' + (v.dev ? '、' + v.dev + (v.dev_released ? '' : ' 開発中') : '') + '）', '', cur === null);
            for (const x of v.versions) add('v' + x.version + '（' + x.tag + '）', x.path, cur === x.version);
            sel.addEventListener('change', function () { location.href = sel.value; });
            const bar = document.querySelector('.right-buttons');
            if (bar) bar.prepend(sel);
            if (cur !== null) {
                const note = document.createElement('div');
                note.className = 'lb-old-version';
                note.style.cssText = 'padding:8px 12px;margin-bottom:16px;border:1px solid var(--quote-border);background:var(--quote-bg);border-radius:4px';
                const a = document.createElement('a');
                a.href = root + rest;
                a.textContent = '最新のドキュメント';
                note.append('これは v' + cur + ' のドキュメントです。', a, 'はこちら。');
                const main = document.querySelector('#content main');
                if (main) main.prepend(note);
            }
        })
        .catch(function () {});
})();
