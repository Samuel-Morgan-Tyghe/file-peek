'use strict';

function localURL(value) {
  try { return new URL(value, 'peek-asset://local/'); }
  catch { return null; }
}

function renderHTML(html) {
  const clean = DOMPurify.sanitize(html, {
    USE_PROFILES: { html: true },
    FORBID_TAGS: ['style', 'form', 'video', 'audio', 'iframe', 'object', 'embed'],
    FORBID_ATTR: ['style', 'srcset']
  });
  const content = document.getElementById('document');
  content.innerHTML = clean;
  const slugs = new Map();
  content.querySelectorAll('h1,h2,h3,h4,h5,h6').forEach(heading => {
    const stem = heading.textContent.toLowerCase().trim().replace(/[^\p{L}\p{N}\s_-]/gu, '').replace(/\s/g, '-');
    const count = slugs.get(stem) || 0;
    slugs.set(stem, count + 1);
    heading.id = count ? `${stem}-${count}` : stem;
  });
  content.querySelectorAll('a[href]').forEach(link => {
    const href = link.getAttribute('href');
    if (href.startsWith('#')) return;
    const url = localURL(href);
    if (!url || !['https:', 'http:', 'mailto:', 'peek-asset:'].includes(url.protocol)) link.removeAttribute('href');
    else link.href = url.href;
    link.rel = 'noreferrer noopener';
  });
  content.querySelectorAll('img').forEach(image => {
    const url = localURL(image.getAttribute('src') || '');
    if (url?.protocol === 'peek-asset:' && url.host === 'local') image.src = url.href;
    else if (!url || !/^data:image\/(png|jpeg|gif|webp);base64,/i.test(url.href)) {
      const placeholder = document.createElement('span');
      placeholder.className = 'image-note';
      placeholder.textContent = image.alt ? `[Remote image: ${image.alt}]` : '[Remote image]';
      image.replaceWith(placeholder);
    }
  });
  window.scrollTo(0, 0);
  return { headings: content.querySelectorAll('h1,h2,h3,h4,h5,h6').length };
}

document.addEventListener('click', event => {
  const href = event.target.closest('a')?.getAttribute('href');
  if (href?.startsWith('#')) {
    event.preventDefault();
    try { document.getElementById(decodeURIComponent(href.slice(1)))?.scrollIntoView(); }
    catch { /* A malformed anchor doesn't need to navigate. */ }
  }
});
