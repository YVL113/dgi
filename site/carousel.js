const advanceInterval = 3_000;
const swipeDistance = 48;

function setupCarousel(carousel) {
  const gallery = carousel.querySelector('.screenshot-gallery');
  const slides = [...gallery.children];
  const controls = carousel.querySelector('.carousel-controls');
  const toggle = controls.querySelector('[data-slideshow-toggle]');
  const caption = controls.querySelector('[data-slide-caption]');
  const title = controls.querySelector('[data-slide-title]');
  const position = controls.querySelector('[data-slide-position]');
  const motion = window.matchMedia?.('(prefers-reduced-motion: reduce)');
  let active = 0;
  let playing = !motion?.matches;
  let visible = !('IntersectionObserver' in window);
  let hovered = gallery.matches(':hover') && window.matchMedia?.('(hover: hover)').matches;
  let timer;
  let touch;

  function schedule() {
    clearTimeout(timer);
    if (playing && visible && !hovered && !touch && !document.hidden) {
      timer = setTimeout(() => advance(1), advanceInterval);
    }
  }

  function advance(direction) {
    const current = slides[active];
    const restoreFocus = current.contains(document.activeElement);
    current.hidden = true;
    delete current.dataset.enter;
    active = (active + direction + slides.length) % slides.length;
    const next = slides[active];
    next.dataset.enter = direction > 0 ? 'next' : 'previous';
    next.hidden = false;
    title.textContent = next.querySelector('a > span').textContent;
    position.textContent = `${active + 1} of ${slides.length}`;
    if (restoreFocus) next.querySelector('a').focus({ preventScroll: true });
    schedule();
  }

  function setPlaying(value) {
    playing = value;
    const action = playing ? 'Pause' : 'Play';
    toggle.textContent = action;
    toggle.setAttribute('aria-label', `${action} slideshow`);
    caption.setAttribute('aria-live', playing ? 'off' : 'polite');
    schedule();
  }

  slides.slice(1).forEach(slide => { slide.hidden = true; });
  carousel.dataset.ready = '';
  carousel.setAttribute('aria-roledescription', 'carousel');
  controls.hidden = false;
  controls.querySelector('[data-slide-previous]').addEventListener('click', () => advance(-1));
  controls.querySelector('[data-slide-next]').addEventListener('click', () => advance(1));
  toggle.addEventListener('click', () => setPlaying(!playing));

  carousel.addEventListener('keydown', event => {
    if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
    if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return;
    event.preventDefault();
    advance(event.key === 'ArrowLeft' ? -1 : 1);
  });
  carousel.addEventListener('focusin', event => {
    if (event.target.matches(':focus-visible') && !carousel.contains(event.relatedTarget)) setPlaying(false);
  });
  gallery.addEventListener('pointerenter', event => {
    if (event.pointerType !== 'mouse') return;
    hovered = true;
    schedule();
  });
  gallery.addEventListener('pointerleave', event => {
    if (event.pointerType !== 'mouse') return;
    hovered = false;
    schedule();
  });
  gallery.addEventListener('touchstart', event => {
    touch = event.touches.length === 1 ? event.touches[0] : undefined;
    schedule();
  }, { passive: true });
  gallery.addEventListener('touchend', event => {
    const start = touch;
    touch = undefined;
    const end = event.changedTouches[0];
    if (start && start.identifier === end?.identifier) {
      const x = end.clientX - start.clientX;
      const y = end.clientY - start.clientY;
      if (Math.abs(x) >= swipeDistance && Math.abs(x) > Math.abs(y)) {
        event.preventDefault();
        advance(x < 0 ? 1 : -1);
        return;
      }
    }
    schedule();
  }, { passive: false });
  gallery.addEventListener('touchcancel', () => {
    touch = undefined;
    schedule();
  }, { passive: true });

  if (!visible) {
    const minimumVisible = .35;
    const observer = new IntersectionObserver(([entry]) => {
      visible = entry.intersectionRatio >= minimumVisible;
      schedule();
    }, { threshold: minimumVisible });
    observer.observe(carousel);
  }
  document.addEventListener('visibilitychange', schedule);
  motion?.addEventListener?.('change', event => { if (event.matches) setPlaying(false); });
  setPlaying(playing);
}

document.querySelectorAll('.preview-carousel').forEach(setupCarousel);
