(function () {
  const state = { section: 0, detail: 0, step: 0 };
  const stacks = Array.from(document.querySelectorAll(".zpres-section-stack"));
  const position = document.getElementById("debug-position");
  const speakerNotesPanel = document.getElementById("zpres-speaker-notes-panel");
  const speakerNotesContent = speakerNotesPanel?.querySelector("[data-speaker-notes-content]");
  let activeSlide = null;
  let transitionSettled = Promise.resolve();
  let navigationRevision = 0;
  let initialReady = null;

  const nextFrame = () => new Promise((resolve) => requestAnimationFrame(resolve));
  const twoFrames = () => new Promise((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(resolve));
  });

  function slidesFor(sectionIndex) {
    return Array.from(stacks[sectionIndex]?.querySelectorAll(".zpres-slide") || []);
  }

  function currentSlide() {
    return slidesFor(state.section)[state.detail] || null;
  }

  function stepElementsFor(slide) {
    return Array.from(slide?.querySelectorAll(".zpres-step.fragment, .zpres-code-line.fragment, .zpres-block-list li.fragment, .zpres-layout-region[data-derivation-role=\"stage\"].fragment") || []);
  }

  function stepCountFor(slide) {
    return stepElementsFor(slide).reduce((max, element) => {
      const index = Number(element.getAttribute("data-step-index") || "0");
      return Number.isSafeInteger(index) && index > max ? index : max;
    }, 0);
  }

  function currentStepCount() {
    return stepCountFor(currentSlide());
  }

  function routeHash(route) {
    const stepSuffix = route.step > 0 ? "/" + route.step : "";
    return "#/" + route.section + "/" + route.detail + stepSuffix;
  }

  function routeSnapshot(section, detail, step) {
    const stack = stacks[section] || null;
    const slide = slidesFor(section)[detail] || null;
    return {
      section,
      detail,
      step,
      hash: routeHash({ section, detail, step }),
      section_index: stack?.dataset.sectionIndex ? Number(stack.dataset.sectionIndex) : null,
      slide_id: slide?.dataset.slideId || "",
      role: slide?.dataset.slideRole || "",
      generated: slide?.dataset.generatedSlide || null,
      step_count: stepCountFor(slide),
    };
  }

  function routes() {
    return stacks.flatMap((stack, section) => {
      const slides = Array.from(stack.querySelectorAll(".zpres-slide"));
      return slides.flatMap((slide, detail) => {
        const stepCount = stepCountFor(slide);
        return Array.from({ length: stepCount + 1 }, (_, step) => routeSnapshot(section, detail, step));
      });
    });
  }

  function current() {
    return routeSnapshot(state.section, state.detail, state.step);
  }

  function validateState(candidate) {
    if (!candidate || typeof candidate !== "object") {
      throw new TypeError("presentation navigation requires a state object");
    }
    const section = candidate.section;
    const detail = candidate.detail;
    const step = candidate.step ?? 0;
    if (!Number.isSafeInteger(section) || !Number.isSafeInteger(detail) || !Number.isSafeInteger(step)) {
      throw new TypeError("presentation section, detail, and step must be safe integers");
    }
    if (section < 0 || section >= stacks.length) {
      throw new RangeError("presentation section " + section + " is outside 0.." + Math.max(0, stacks.length - 1));
    }
    const slides = slidesFor(section);
    if (detail < 0 || detail >= slides.length) {
      throw new RangeError("presentation detail " + detail + " is outside 0.." + Math.max(0, slides.length - 1) + " for section " + section);
    }
    const stepCount = stepCountFor(slides[detail]);
    if (step < 0 || step > stepCount) {
      throw new RangeError("presentation step " + step + " is outside 0.." + stepCount + " for section " + section + ", detail " + detail);
    }
    return { section, detail, step };
  }

  function applyFragments() {
    stacks.forEach((stack, stackIndex) => {
      slidesFor(stackIndex).forEach((slide, slideIndex) => {
        const isActive = stackIndex === state.section && slideIndex === state.detail;
        stepElementsFor(slide).forEach((step) => {
          const index = Number(step.getAttribute("data-step-index") || "0");
          const visible = isActive && index <= state.step;
          step.classList.toggle("is-visible", visible);
          step.setAttribute("data-step-state", !visible ? "future" : index === state.step ? "active" : "complete");
          if (visible && index === state.step) step.setAttribute("aria-current", "step");
          else step.removeAttribute("aria-current");
        });
        const stepCount = stepCountFor(slide);
        const currentStep = isActive ? state.step : 0;
        slide.setAttribute("data-current-step", String(currentStep));
        slide.setAttribute("data-step-count", String(stepCount));
        const progress = slide.querySelector("[data-zpres-step-progress]");
        if (progress) progress.textContent = "Step " + currentStep + " of " + stepCount;
        applyStepGates(slide, isActive);
      });
    });
  }

  function applyStepGates(slide, isActive) {
    const steps = Array.from(slide?.querySelectorAll(".zpres-block-steps") || []);
    slide.querySelectorAll(".is-step-gated").forEach((block) => {
      block.classList.remove("is-step-gated");
      block.removeAttribute("aria-hidden");
    });
    if (!isActive) return;
    steps.forEach((stepBlock) => {
      const stepCount = Number(stepBlock.getAttribute("data-step-count") || "0");
      if (!stepCount || state.step >= stepCount) return;
      let sibling = stepBlock.nextElementSibling;
      while (sibling) {
        sibling.classList.add("is-step-gated");
        sibling.setAttribute("aria-hidden", "true");
        sibling = sibling.nextElementSibling;
      }
    });
  }

  function updateSpeakerNotes() {
    if (!speakerNotesPanel || !speakerNotesContent) return;
    const notes = Array.from(currentSlide()?.querySelectorAll(".zpres-block-speaker-notes .zpres-speaker-notes-body") || []);
    if (!notes.length) {
      speakerNotesContent.textContent = "No speaker notes.";
      speakerNotesPanel.setAttribute("data-empty", "true");
      return;
    }
    speakerNotesPanel.removeAttribute("data-empty");
    speakerNotesContent.innerHTML = notes.map((note) => note.innerHTML).join("");
    applySpeakerNoteClickMarkers();
  }

  function applySpeakerNoteClickMarkers() {
    if (!speakerNotesContent) return;
    const markers = Array.from(speakerNotesContent.querySelectorAll("[data-note-click-index]"));
    markers.forEach((marker) => {
      const index = Number(marker.getAttribute("data-note-click-index") || "0");
      marker.classList.toggle("is-pending", index > state.step);
      marker.classList.toggle("is-active", index === state.step);
      marker.classList.toggle("is-complete", index > 0 && index < state.step);
    });
  }

  function toggleSpeakerNotes() {
    if (!speakerNotesPanel) return;
    speakerNotesPanel.hidden = !speakerNotesPanel.hidden;
    updateSpeakerNotes();
  }

  function isTypingTarget(target) {
    return target instanceof Element && Boolean(target.closest(
      "a[href], button, input, textarea, select, summary, iframe, audio[controls], video[controls], [contenteditable='true'], [tabindex]:not([tabindex='-1'])",
    ));
  }

  function beginTransition(previousActive) {
    if (!activeSlide || activeSlide === previousActive || activeSlide.getAttribute("data-transition") === "none") {
      activeSlide?.classList.remove("is-entering");
      return Promise.resolve();
    }
    const transitionSlide = activeSlide;
    transitionSlide.classList.remove("is-entering");
    void transitionSlide.offsetWidth;
    transitionSlide.classList.add("is-entering");
    return nextFrame().then(async () => {
      if (activeSlide !== transitionSlide) return;
      const animations = finiteAnimationsFor(activeSlide);
      await Promise.allSettled(animations.map((animation) => animation.finished));
    }).finally(() => transitionSlide.classList.remove("is-entering"));
  }

  function retainsLeavingSlide(slide) {
    if (!slide) return false;
    return getComputedStyle(slide).getPropertyValue("--zpres-retain-leaving-slide").trim() === "1";
  }

  function routeDirection(previous, next) {
    if (!previous) return "forward";
    const previousIndex = previous.section * 10000 + previous.detail;
    const nextIndex = next.section * 10000 + next.detail;
    return nextIndex < previousIndex ? "backward" : "forward";
  }

  function clearLeavingSlide(slide, token) {
    if (!slide || (token && slide.dataset.zpresTransitionToken !== token)) return;
    slide.classList.remove("is-leaving");
    slide.removeAttribute("data-zpres-transition-direction");
    slide.removeAttribute("data-zpres-transition-token");
    slide.removeAttribute("aria-hidden");
    slide.removeAttribute("inert");
    const stack = slide.closest(".zpres-section-stack");
    if (stack && !stack.querySelector(".zpres-slide.is-leaving")) stack.classList.remove("is-leaving");
  }

  function clearStaleLeavingSlides() {
    document.querySelectorAll(".zpres-slide.is-leaving").forEach((slide) => clearLeavingSlide(slide));
  }

  function beginSheetTransition(previousActive, previousRoute, nextRoute, revision) {
    clearStaleLeavingSlides();
    if (!activeSlide || activeSlide === previousActive || !retainsLeavingSlide(activeSlide)) {
      return beginTransition(previousActive);
    }
    const direction = routeDirection(previousRoute, nextRoute);
    const token = String(revision);
    activeSlide.dataset.zpresTransitionDirection = direction;
    activeSlide.classList.remove("is-entering");
    void activeSlide.offsetWidth;
    activeSlide.classList.add("is-entering");
    if (previousActive) {
      previousActive.dataset.zpresTransitionDirection = direction;
      previousActive.dataset.zpresTransitionToken = token;
      previousActive.classList.remove("is-entering");
      previousActive.classList.add("is-leaving");
      previousActive.setAttribute("aria-hidden", "true");
      previousActive.setAttribute("inert", "");
      previousActive.closest(".zpres-section-stack")?.classList.add("is-leaving");
    }
    const enteringSlide = activeSlide;
    return nextFrame().then(async () => {
      const animations = [
        ...finiteAnimationsFor(enteringSlide),
        ...finiteAnimationsFor(previousActive),
      ];
      await Promise.allSettled(animations.map((animation) => animation.finished));
    }).finally(() => {
      enteringSlide.classList.remove("is-entering");
      enteringSlide.removeAttribute("data-zpres-transition-direction");
      clearLeavingSlide(previousActive, token);
    });
  }

  function finiteAnimationsFor(root) {
    if (!root || typeof root.getAnimations !== "function") return [];
    return root.getAnimations({ subtree: true }).filter((animation) => {
      const timing = animation.effect?.getComputedTiming();
      return timing?.iterations !== Infinity;
    });
  }

  function applyState(candidate, revision = navigationRevision) {
    const next = validateState(candidate);
    const previousRoute = activeSlide ? { ...state } : null;
    state.section = next.section;
    state.detail = next.detail;
    state.step = next.step;
    const previousActive = activeSlide;
    stacks.forEach((stack, stackIndex) => {
      stack.classList.toggle("is-active", stackIndex === state.section);
      slidesFor(stackIndex).forEach((slide, slideIndex) => {
        slide.classList.toggle("is-active", stackIndex === state.section && slideIndex === state.detail);
        if (!slide.classList.contains("is-active")) slide.classList.remove("is-entering");
      });
    });
    activeSlide = currentSlide();
    transitionSettled = beginSheetTransition(previousActive, previousRoute, next, revision);
    applyFragments();
    if (position) {
      const stepCount = currentStepCount();
      const stepLabel = stepCount ? " / step " + state.step + " of " + stepCount : "";
      position.textContent = "section " + (state.section + 1) + " / slide " + (state.detail + 1) + stepLabel;
    }
    updateSpeakerNotes();
    if (location.hash !== routeHash(state)) {
      history.replaceState(null, "", routeHash(state));
    }
    return transitionSettled;
  }

  async function settleState(revision, transition) {
    await nextFrame();
    if (typeof window.zpresAutoscaleAll === "function") {
      await Promise.resolve(window.zpresAutoscaleAll());
    }
    await transition;
    await nextFrame();
    await Promise.allSettled(
      finiteAnimationsFor(activeSlide).map((animation) => animation.finished),
    );
    await twoFrames();
    if (revision !== navigationRevision) {
      const error = new Error("presentation navigation was superseded by a newer state");
      error.name = "AbortError";
      throw error;
    }
    return current();
  }

  async function navigate(candidate) {
    const next = validateState(candidate);
    const revision = ++navigationRevision;
    const transition = applyState(next, revision);
    return settleState(revision, transition);
  }

  function stateFromHash(hash) {
    const match = hash.match(/^#\/(\d+)\/(\d+)(?:\/(\d+))?$/);
    if (!match) return null;
    return {
      section: Number(match[1]),
      detail: Number(match[2]),
      step: match[3] ? Number(match[3]) : 0,
    };
  }

  function rememberNavigationError(error) {
    window.zpresPresentationLastError = {
      name: error?.name || "Error",
      message: error?.message || String(error),
    };
  }

  function requestNavigation(candidate) {
    navigate(candidate).catch((error) => {
      if (error?.name !== "AbortError") rememberNavigationError(error);
    });
  }

  function move(deltaSection, deltaDetail) {
    if (deltaSection !== 0) {
      const nextSection = state.section + deltaSection;
      if (nextSection < 0 || nextSection >= stacks.length) {
        requestNavigation(state);
        return;
      }
      requestNavigation({ section: nextSection, detail: 0, step: 0 });
    } else {
      const nextDetail = state.detail + deltaDetail;
      if (nextDetail < 0 || nextDetail >= slidesFor(state.section).length) {
        requestNavigation(state);
        return;
      }
      requestNavigation({ section: state.section, detail: nextDetail, step: 0 });
    }
  }

  function nextStepOrSection() {
    if (state.step < currentStepCount()) {
      requestNavigation({ section: state.section, detail: state.detail, step: state.step + 1 });
    } else {
      move(1, 0);
    }
  }

  function previousStepOrSection() {
    if (state.step > 0) {
      requestNavigation({ section: state.section, detail: state.detail, step: state.step - 1 });
    } else {
      move(-1, 0);
    }
  }

  document.addEventListener("keydown", (event) => {
    if (isTypingTarget(event.target)) return;
    let handled = true;
    if (event.key === "n" || event.key === "N") {
      toggleSpeakerNotes();
    } else if (event.key === "ArrowRight" || event.key === "PageDown" || event.key === " ") {
      nextStepOrSection();
    } else if (event.key === "ArrowLeft" || event.key === "PageUp") {
      previousStepOrSection();
    } else if (event.key === "ArrowDown") {
      move(0, 1);
    } else if (event.key === "ArrowUp") {
      move(0, -1);
    } else {
      handled = false;
    }
    if (handled) event.preventDefault();
  });

  document.querySelectorAll("[data-nav]").forEach((button) => {
    button.addEventListener("click", () => {
      const action = button.getAttribute("data-nav");
      if (action === "prev-section") move(-1, 0);
      if (action === "next-section") move(1, 0);
      if (action === "prev-detail") move(0, -1);
      if (action === "next-detail") move(0, 1);
    });
  });

  document.addEventListener("click", (event) => {
    if (!(event.target instanceof Element)) return;
    const link = event.target.closest("a[data-zpres-footnote-target]");
    if (!link || !currentSlide()?.contains(link)) return;
    const note = document.getElementById(link.dataset.zpresFootnoteTarget);
    if (!note || !currentSlide()?.contains(note)) return;
    event.preventDefault();
    note.setAttribute("tabindex", "-1");
    note.focus({ preventScroll: true });
  });

  document.addEventListener("ended", (event) => {
    const media = event.target;
    if (!(media instanceof HTMLMediaElement)) return;
    const block = media.closest(".zpres-block-media[data-media-autoadvance=\"true\"]");
    if (!block || !currentSlide()?.contains(block)) return;
    nextStepOrSection();
  }, true);

  window.addEventListener("hashchange", () => {
    const requested = stateFromHash(location.hash);
    if (!requested) {
      const error = new RangeError("invalid presentation route " + location.hash);
      history.replaceState(null, "", routeHash(state));
      rememberNavigationError(error);
      return;
    }
    navigate(requested).catch((error) => {
      history.replaceState(null, "", routeHash(state));
      rememberNavigationError(error);
    });
  });

  window.zpresPresentation = Object.freeze({
    routes,
    navigate,
    current,
    get ready() {
      return initialReady;
    },
  });

  const requestedInitialState = location.hash
    ? stateFromHash(location.hash)
    : { section: 0, detail: 0, step: 0 };
  if (requestedInitialState) {
    initialReady = navigate(requestedInitialState).catch((error) => {
      applyState({ section: 0, detail: 0, step: 0 });
      throw error;
    });
  } else {
    applyState({ section: 0, detail: 0, step: 0 });
    initialReady = Promise.reject(new RangeError("invalid presentation route " + location.hash));
  }
  initialReady.catch(rememberNavigationError);
  const renderPromises = window.zpresStaticRenderPromises == null
    ? []
    : Array.isArray(window.zpresStaticRenderPromises)
      ? window.zpresStaticRenderPromises
      : [window.zpresStaticRenderPromises];
  renderPromises.push(initialReady);
  window.zpresStaticRenderPromises = renderPromises;
})();
