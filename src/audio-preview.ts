import { button, icon } from './ui';

function formatTime(seconds: number) {
  const wholeSeconds = Math.floor(seconds);
  const minutes = Math.floor(wholeSeconds / 60);
  const remainder = String(wholeSeconds % 60).padStart(2, '0');
  if (minutes < 60) return `${minutes}:${remainder}`;
  return `${Math.floor(minutes / 60)}:${String(minutes % 60).padStart(2, '0')}:${remainder}`;
}

function range(label: string, max: string, step: string) {
  const input = document.createElement('input');
  input.type = 'range';
  input.min = '0';
  input.max = max;
  input.step = step;
  input.setAttribute('aria-label', label);
  return input;
}

export function mountAudio(container: HTMLElement, bytes: ArrayBuffer, mime: string, onError: () => void) {
  const audio = document.createElement('audio');
  audio.hidden = true;
  audio.preload = 'metadata';
  const play = button('', { icon: 'play', variant: 'primary', size: 'icon', ariaLabel: 'Play audio' });
  const mute = button('', { icon: 'volume-2', variant: 'ghost', size: 'icon', ariaLabel: 'Mute audio' });
  const seek = range('Audio position', '0', 'any');
  const volume = range('Audio volume', '100', '1');
  const elapsed = document.createElement('span');
  const duration = document.createElement('span');
  const times = document.createElement('div');
  times.className = 'audio-times';
  times.append(elapsed, duration);

  const timeline = document.createElement('div');
  timeline.className = 'audio-timeline';
  timeline.append(seek, times);
  const sound = document.createElement('div');
  sound.className = 'audio-volume';
  sound.append(mute, volume);
  const controls = document.createElement('div');
  controls.className = 'audio-controls panel';
  controls.setAttribute('role', 'group');
  controls.setAttribute('aria-label', 'Audio preview');
  controls.append(play, timeline, sound, audio);

  const life = new AbortController();
  const options = { signal: life.signal };
  const url = URL.createObjectURL(new Blob([bytes], { type: mime }));

  function updatePlayback() {
    play.disabled = audio.readyState < HTMLMediaElement.HAVE_METADATA;
    const playing = !audio.paused && !audio.ended;
    const label = playing ? 'Pause audio' : 'Play audio';
    play.setAttribute('aria-label', label);
    play.title = label;
    play.replaceChildren(icon(playing ? 'pause' : 'play'));
  }

  function updateTimeline() {
    const length = Number.isFinite(audio.duration) && audio.duration > 0 ? audio.duration : 0;
    seek.disabled = length === 0;
    seek.max = String(length);
    seek.value = String(Math.min(audio.currentTime, length));
    elapsed.textContent = formatTime(audio.currentTime);
    duration.textContent = length ? formatTime(length) : '–:––';
    seek.setAttribute('aria-valuetext', length ? elapsed.textContent + ' of ' + duration.textContent : elapsed.textContent);
  }

  function updateVolume() {
    const percent = audio.muted ? 0 : Math.round(audio.volume * 100);
    volume.value = String(percent);
    volume.setAttribute('aria-valuetext', percent + '%');
    const label = audio.muted ? 'Unmute audio' : 'Mute audio';
    mute.setAttribute('aria-label', label);
    mute.title = label;
    mute.replaceChildren(icon(audio.muted ? 'volume-x' : 'volume-2'));
  }

  function dispose() {
    if (life.signal.aborted) return;
    life.abort();
    audio.pause();
    audio.removeAttribute('src');
    audio.load();
    URL.revokeObjectURL(url);
  }

  function fail() {
    if (life.signal.aborted) return;
    dispose();
    onError();
  }

  play.addEventListener('click', () => {
    if (audio.paused || audio.ended) {
      void audio.play().catch((error: unknown) => {
        if (!(error instanceof DOMException && error.name === 'AbortError')) fail();
      });
    } else audio.pause();
  }, options);
  mute.addEventListener('click', () => { audio.muted = !audio.muted; }, options);
  volume.addEventListener('input', () => {
    const level = volume.valueAsNumber;
    if (level > 0) audio.volume = level / 100;
    audio.muted = level === 0;
  }, options);
  seek.addEventListener('input', () => {
    if (!seek.disabled) audio.currentTime = seek.valueAsNumber;
    updateTimeline();
  }, options);
  for (const event of ['loadedmetadata', 'play', 'pause', 'ended']) {
    audio.addEventListener(event, updatePlayback, options);
  }
  for (const event of ['durationchange', 'loadedmetadata', 'timeupdate']) {
    audio.addEventListener(event, updateTimeline, options);
  }
  audio.addEventListener('volumechange', updateVolume, options);
  audio.addEventListener('error', fail, options);
  updatePlayback();
  updateTimeline();
  updateVolume();
  container.replaceChildren(controls);
  audio.src = url;
  return dispose;
}
