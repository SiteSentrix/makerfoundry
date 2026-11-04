// MakerFoundry note counter (issue #80; the owner agreed to it, 2026-11-03): under each
// <textarea data-count="500"> it shows "N / 500" as you type. Our own code, MIT OR Apache-2.0,
// with no libraries. Without JavaScript the box's maxlength and the server's check still hold;
// this only shows the count.
//
// It counts as StitchHub counts a note (src/sync-orders.mjs, `text`): control characters and
// runs of white space are one space, the ends trimmed, and the length is JavaScript's own.
(function () {
  'use strict';
  function hubLength(v) {
    return v.replace(/[\u0000-\u001f\u007f]+/g, ' ').replace(/\s+/g, ' ').trim().length;
  }
  var boxes = document.querySelectorAll('textarea[data-count]');
  Array.prototype.forEach.call(boxes, function (box) {
    var max = parseInt(box.getAttribute('data-count'), 10);
    if (!(max > 0)) return;
    var shown = document.createElement('small');
    shown.className = 'note-count';
    box.parentNode.insertBefore(shown, box.nextSibling);
    function update() {
      var n = hubLength(box.value);
      shown.textContent = n + ' / ' + max;
      shown.classList.toggle('over', n > max);
    }
    box.addEventListener('input', update);
    update();
  });
})();
