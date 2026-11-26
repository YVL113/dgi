const advanceInterval = 3_000;
const swipeDistance = 48;

function setupCarousel(carousel) {
  const gallery = carousel.querySelector('.screenshot-gallery');
  const slides = [...gallery.children];
  const motion = window.matchMedia?.('(prefers-reduced-motion: reduce)');
  let active = 0;
  let visible = !('IntersectionObserver' in window);
  let hovered = gallery.matches(':hover') && window.matchMedia?.('(hover: hover)').matches;
  let timer;
  let touch;

  function schedule() {
    clearTimeout(timer);
    if (visible && !motion?.matches && !hovered && !touch && !document.hidden && !gallery.matches(':focus-within')) {
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
    if (restoreFocus) next.querySelector('a').focus({ preventScroll: true });
    schedule();
  }

  slides.slice(1).forEach(slide => { slide.hidden = true; });
  carousel.dataset.ready = '';
  carousel.setAttribute('aria-roledescription', 'carousel');

  carousel.addEventListener('keydown', event => {
    if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
    if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return;
    event.preventDefault();
    advance(event.key === 'ArrowLeft' ? -1 : 1);
  });
  gallery.addEventListener('focusin', schedule);
  gallery.addEventListener('focusout', () => queueMicrotask(schedule));
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
  motion?.addEventListener?.('change', schedule);
  schedule();
}

document.querySelectorAll('.preview-carousel').forEach(setupCarousel);
