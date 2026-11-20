// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

// Size + theme from the URL hash: "#1600x900", "#1200x630-dark", "#1080x1080".
// 1200x630 is laid out at 1600x840 and zoomed 0.75 so all three landscape/square cards share one layout.
(function () {
  var h = (location.hash || '#1600x900').slice(1).split('-');
  var sizes = { '1600x900': [1600, 900, 1], '1200x630': [1600, 840, 0.75], '1080x1080': [1080, 1080, 1] };
  var m = sizes[h[0]] || sizes['1600x900'];
  var de = document.documentElement, card = document.querySelector('.card');
  de.style.width = document.body.style.width = Math.round(m[0] * m[2]) + 'px';
  de.style.height = document.body.style.height = Math.round(m[1] * m[2]) + 'px';
  card.style.width = m[0] + 'px'; card.style.height = m[1] + 'px'; card.style.zoom = m[2];
  if (h[1] === 'dark') document.body.classList.add('dark');
  if (h[0] === '1200x630') document.body.classList.add('short');
  if (h[0] === '1080x1080') document.body.classList.add('sq');
})();
