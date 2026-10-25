// Home page: a rotating hero headline, and pictures that move as you scroll.
// Everything here is decoration. Without JavaScript, or with reduced motion
// turned on, the page shows the first headline and still pictures.
(function () {
  var reduceMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches;

  // ---------------------------------------------------------------- headline
  // The first line is the one in the HTML; the rest follow in a random order.
  var LINES = [
    'A writing desk for working a draft over.',
    'Keep every version of every scene.',
    'Lay one draft over another.',
    'Hear your draft read back to you.',
    'Talk, and the words land on the page.',
    'Your manuscript, in a folder you own.',
    'Free to write. $5 to publish.',
    'No account. No subscription. No cloud.'
  ];
  var HOLD_MS = 3800;

  function shuffle(list) {
    for (var i = list.length - 1; i > 0; i--) {
      var j = Math.floor(Math.random() * (i + 1));
      var t = list[i]; list[i] = list[j]; list[j] = t;
    }
    return list;
  }

  function rotateHeadline() {
    var h1 = document.getElementById('hero-title');
    if (!h1) return;
    var order = [h1.textContent.trim()].concat(shuffle(LINES.slice(1)));

    // Screen readers and search engines keep the one steady headline. The
    // rotating copies sit in a single grid cell, so the heading is always as
    // tall as its longest line and nothing below it jumps.
    var steady = document.createElement('span');
    steady.className = 'sr-only';
    steady.textContent = order[0];
    var stack = document.createElement('span');
    stack.className = 'hero-rotator';
    stack.setAttribute('aria-hidden', 'true');
    var spans = order.map(function (text, i) {
      var s = document.createElement('span');
      s.textContent = text;
      if (i === 0) s.className = 'is-on';
      stack.appendChild(s);
      return s;
    });
    h1.textContent = '';
    h1.appendChild(steady);
    h1.appendChild(stack);

    var current = 0;
    var timer = null;
    function next() {
      spans[current].classList.remove('is-on');
      current = (current + 1) % spans.length;
      spans[current].classList.add('is-on');
    }
    function start() { if (!timer) timer = setInterval(next, HOLD_MS); }
    function stop() { clearInterval(timer); timer = null; }

    // Hold still while the pointer is on it, and while the tab is hidden.
    h1.addEventListener('mouseenter', stop);
    h1.addEventListener('mouseleave', start);
    document.addEventListener('visibilitychange', function () {
      document.hidden ? stop() : start();
    });
    start();
  }

  // ---------------------------------------------------------------- reveal
  // The editor, cards and list items rise into place the first time they
  // scroll into view; the hero's editor does it on load.
  function reveal() {
    var targets = document.querySelectorAll(
      '.hero-media, .feature-card, .home-band .check-list > li, .price-card, .ask-card, .faq__item'
    );
    if (!('IntersectionObserver' in window)) return;
    document.documentElement.classList.add('motion');

    // Once risen, hand the element back to its own styles (card hover lifts).
    function done(el) {
      el.classList.add('is-done');
      el.style.transitionDelay = '';
    }
    var seen = new IntersectionObserver(function (entries) {
      entries.forEach(function (entry) {
        if (!entry.isIntersecting) return;
        var el = entry.target;
        el.classList.add('is-in');
        seen.unobserve(el);
        setTimeout(function () { done(el); }, 1400);
      });
    }, { rootMargin: '0px 0px -12% 0px' });
    targets.forEach(function (el) {
      // Siblings in a row arrive one after another.
      var parent = el.parentElement;
      var index = Array.prototype.indexOf.call(parent.children, el);
      if (parent.children.length > 1) el.style.transitionDelay = (index % 3) * 90 + 'ms';
      seen.observe(el);
    });
  }

  // ---------------------------------------------------------------- parallax
  // A few pictures drift at their own pace as the page scrolls past them.
  // depth is pixels of travel across the viewport; negative moves up.
  function parallax() {
    var layers = [
      { el: document.querySelector('.hero-media .mock'), depth: -40 },
      { el: document.querySelector('.ask-card__sheets img'), depth: -18 },
      { el: document.querySelector('.price-card--featured .price-card__tag'), depth: -10 }
    ].filter(function (l) { return l.el; });
    if (!layers.length) return;

    var ticking = false;
    function update() {
      ticking = false;
      var vh = window.innerHeight;
      layers.forEach(function (l) {
        var box = l.el.getBoundingClientRect();
        if (box.bottom < -200 || box.top > vh + 200) return;
        // -1 when the element's centre is at the bottom of the screen, 1 at the top.
        var progress = 1 - ((box.top + box.height / 2) / vh) * 2;
        l.el.style.transform = 'translate3d(0,' + (progress * l.depth).toFixed(1) + 'px,0)';
      });
    }
    function request() {
      if (!ticking) { ticking = true; requestAnimationFrame(update); }
    }
    window.addEventListener('scroll', request, { passive: true });
    window.addEventListener('resize', request);
    update();
  }

  document.addEventListener('DOMContentLoaded', function () {
    if (reduceMotion) return;
    rotateHeadline();
    reveal();
    parallax();
  });
})();
