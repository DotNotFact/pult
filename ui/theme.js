// Тема до первой отрисовки, чтобы не мигал светлый фон. Отдельным файлом: CSP запрещает встроенные скрипты.
try {
  const t = JSON.parse(localStorage.getItem('pult.theme'));
  if (t) document.documentElement.dataset.theme = t;
} catch { /* нет сохранённой темы */ }
