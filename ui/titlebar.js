// Свой заголовок окна (окно без рамки Windows): перетаскивание, свернуть, развернуть, закрыть.
// Зачем: у системного заголовка двойной щелчок по значку закрывает окно, и он не в стиле Пульта.
// Здесь же запрет контекстного меню WebView («Назад», «Обновить», «Проверить») - кроме полей ввода.
(() => {
  const win = () => window.__TAURI__?.window?.getCurrentWindow?.();
  const tb = document.getElementById('titlebar');
  const T = (k, d) => window.S?.kb?.ui?.titlebar?.[k] || d;

  // меню остаётся там, где оно нужно: поля ввода и выделенный текст («Копировать»)
  document.addEventListener('contextmenu', (e) => {
    if (e.target.closest('input, textarea, [contenteditable="true"]')) return;
    if (String(window.getSelection?.() || '').trim()) return;
    e.preventDefault();
  });

  if (!tb) return;
  // страховка: рамка Windows поверх своего заголовка - два заголовка; убрать её, если кто-то вернул
  win()?.isDecorated?.().then((d) => { if (d) win().setDecorations(false); }).catch(() => {});
  const ico = {
    min: '<svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true"><path d="M1 5.5h8" stroke="currentColor" stroke-width="1.1"/></svg>',
    max: '<svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true"><rect x="1.5" y="1.5" width="7" height="7" rx="1" fill="none" stroke="currentColor" stroke-width="1.1"/></svg>',
    restore: '<svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true"><rect x="1.5" y="3" width="5.5" height="5.5" rx="1" fill="none" stroke="currentColor" stroke-width="1.1"/><path d="M3.5 3V1.8h4.7v4.7H7" fill="none" stroke="currentColor" stroke-width="1.1"/></svg>',
    close: '<svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true"><path d="M1.5 1.5l7 7M8.5 1.5l-7 7" stroke="currentColor" stroke-width="1.1"/></svg>',
  };
  // верхний край: у окна без рамки Tauri тянет только бока и низ (в тени), верх - эта полоса и углы
  tb.innerHTML = `<div class="tb-edge n" data-rz="North"></div><div class="tb-edge nw" data-rz="NorthWest"></div><div class="tb-edge ne" data-rz="NorthEast"></div>
    <img class="tb-logo" src="img/logo.png" alt="" data-tauri-drag-region>
    <span class="tb-title" data-tauri-drag-region>Пульт</span>
    <div class="tb-btns">
      <button class="tb-btn" data-tb="min" tabindex="-1">${ico.min}</button>
      <button class="tb-btn" data-tb="max" tabindex="-1">${ico.max}</button>
      <button class="tb-btn close" data-tb="close" tabindex="-1">${ico.close}</button>
    </div>`;
  const titles = () => {
    tb.querySelector('[data-tb=min]').title = T('min', 'Свернуть');
    tb.querySelector('[data-tb=max]').title = tb.classList.contains('max') ? T('restore', 'Восстановить') : T('max', 'Развернуть');
    tb.querySelector('[data-tb=close]').title = T('close', 'Закрыть');
  };
  titles();
  tb.addEventListener('mousedown', (e) => {
    const z = e.target.closest('[data-rz]');
    if (!z || e.button !== 0 || tb.classList.contains('max')) return;
    e.preventDefault();
    win()?.startResizeDragging?.(z.dataset.rz).catch(() => {});
  });
  tb.addEventListener('click', (e) => {
    const b = e.target.closest('[data-tb]');
    if (!b) return;
    const w = win();
    if (!w) return;
    if (b.dataset.tb === 'min') w.minimize();
    else if (b.dataset.tb === 'max') w.toggleMaximize();
    else w.close(); // крестик: Пульт закроется или уйдёт в трей - как в настройках
  });
  const syncMax = async () => {
    const w = win();
    if (!w) return;
    const m = await w.isMaximized().catch(() => false);
    tb.classList.toggle('max', m);
    tb.querySelector('[data-tb=max]').innerHTML = m ? ico.restore : ico.max;
    titles();
  };
  win()?.onResized?.(() => syncMax());
  syncMax();

  window.TitleBar = {
    // виджет прячет заголовок: его шапка сама служит местом для перетаскивания
    hide() { document.documentElement.classList.add('no-tb'); },
    show() { document.documentElement.classList.remove('no-tb'); },
    setTitle(name) { tb.querySelector('.tb-title').textContent = name ? `Пульт · ${name}` : 'Пульт'; titles(); },
  };
})();
