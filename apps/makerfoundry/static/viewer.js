// MakerFoundry 3D viewer (MF-46): draws a model's mesh with WebGL. Drag to turn it, scroll or
// pinch to zoom. Our own code, MIT OR Apache-2.0, with no libraries. Without JavaScript or WebGL
// the page keeps its server-drawn pictures; this only adds the view you can turn.
//
// The mesh (made by the server, src/model3d/viewer.rs): "MFM1", a u32 triangle count, the
// model's box (6 x f32), then 9 x u16 a triangle, each a coordinate inside the box.
(function () {
  'use strict';
  var box = document.querySelector('.viewer[data-mesh]');
  if (!box || !window.fetch) return;
  var canvas = box.querySelector('canvas');
  var gl = canvas && canvas.getContext('webgl', { antialias: true, premultipliedAlpha: false });
  if (!gl) return;

  fetch(box.getAttribute('data-mesh'), { credentials: 'same-origin' })
    .then(function (r) { if (!r.ok) throw new Error('mesh ' + r.status); return r.arrayBuffer(); })
    .then(function (buffer) { start(decode(buffer)); })
    .catch(function () { /* the pictures stay */ });

  function decode(buffer) {
    var view = new DataView(buffer);
    if (buffer.byteLength < 32 || view.getUint32(0, false) !== 0x4d464d31) throw new Error('not a mesh');
    var n = view.getUint32(4, true);
    if (buffer.byteLength !== 32 + n * 18) throw new Error('mesh size');
    var lo = [], hi = [], i;
    for (i = 0; i < 3; i++) { lo.push(view.getFloat32(8 + i * 4, true)); hi.push(view.getFloat32(20 + i * 4, true)); }
    var centre = [(lo[0] + hi[0]) / 2, (lo[1] + hi[1]) / 2, (lo[2] + hi[2]) / 2];
    var radius = Math.max(1e-6, Math.hypot(hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]) / 2);
    var positions = new Float32Array(n * 9), normals = new Float32Array(n * 9);
    var at = 32;
    for (var t = 0; t < n; t++) {
      var p = [];
      for (var c = 0; c < 3; c++) {
        for (var a = 0; a < 3; a++) {
          var v = lo[a] + view.getUint16(at, true) / 65535 * (hi[a] - lo[a]) - centre[a];
          at += 2;
          positions[t * 9 + c * 3 + a] = v;
          p.push(v);
        }
      }
      // a flat normal for the triangle
      var ux = p[3] - p[0], uy = p[4] - p[1], uz = p[5] - p[2];
      var vx = p[6] - p[0], vy = p[7] - p[1], vz = p[8] - p[2];
      var nx = uy * vz - uz * vy, ny = uz * vx - ux * vz, nz = ux * vy - uy * vx;
      var len = Math.hypot(nx, ny, nz) || 1;
      for (c = 0; c < 3; c++) {
        normals[t * 9 + c * 3] = nx / len; normals[t * 9 + c * 3 + 1] = ny / len; normals[t * 9 + c * 3 + 2] = nz / len;
      }
    }
    return { count: n * 3, positions: positions, normals: normals, radius: radius };
  }

  function shader(type, source) {
    var s = gl.createShader(type);
    gl.shaderSource(s, source);
    gl.compileShader(s);
    if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s));
    return s;
  }

  // 4x4 matrices, column by column, as WebGL takes them
  function multiply(a, b) {
    var out = new Float32Array(16);
    for (var col = 0; col < 4; col++) for (var row = 0; row < 4; row++) {
      var sum = 0;
      for (var k = 0; k < 4; k++) sum += a[k * 4 + row] * b[col * 4 + k];
      out[col * 4 + row] = sum;
    }
    return out;
  }
  function rotateX(r) { var c = Math.cos(r), s = Math.sin(r); return new Float32Array([1, 0, 0, 0, 0, c, s, 0, 0, -s, c, 0, 0, 0, 0, 1]); }
  function rotateZ(r) { var c = Math.cos(r), s = Math.sin(r); return new Float32Array([c, s, 0, 0, -s, c, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]); }
  function perspective(fov, aspect, near, far) {
    var f = 1 / Math.tan(fov / 2), nf = 1 / (near - far);
    return new Float32Array([f / aspect, 0, 0, 0, 0, f, 0, 0, 0, 0, (far + near) * nf, -1, 0, 0, 2 * far * near * nf, 0]);
  }

  function start(mesh) {
    var program = gl.createProgram();
    gl.attachShader(program, shader(gl.VERTEX_SHADER,
      'attribute vec3 p; attribute vec3 n; uniform mat4 mvp; uniform mat3 turn; varying vec3 vn;' +
      'void main() { vn = turn * n; gl_Position = mvp * vec4(p, 1.0); }'));
    gl.attachShader(program, shader(gl.FRAGMENT_SHADER,
      'precision mediump float; varying vec3 vn;' +
      'void main() { vec3 light = normalize(vec3(-0.45, 0.6, 0.65));' +
      ' float shade = 0.3 + 0.7 * abs(dot(normalize(vn), light));' +
      ' gl_FragColor = vec4(vec3(0.376, 0.518, 0.69) * shade, 1.0); }'));
    gl.linkProgram(program);
    if (!gl.getProgramParameter(program, gl.LINK_STATUS)) return;
    gl.useProgram(program);
    function attribute(name, data) {
      var b = gl.createBuffer();
      gl.bindBuffer(gl.ARRAY_BUFFER, b);
      gl.bufferData(gl.ARRAY_BUFFER, data, gl.STATIC_DRAW);
      var at = gl.getAttribLocation(program, name);
      gl.enableVertexAttribArray(at);
      gl.vertexAttribPointer(at, 3, gl.FLOAT, false, 0, 0);
    }
    attribute('p', mesh.positions);
    attribute('n', mesh.normals);
    var mvpAt = gl.getUniformLocation(program, 'mvp'), turnAt = gl.getUniformLocation(program, 'turn');
    gl.enable(gl.DEPTH_TEST);
    gl.clearColor(0, 0, 0, 0);

    var yaw = -0.6, pitch = 0.5, distance = mesh.radius * 4.2;
    function draw() {
      var ratio = window.devicePixelRatio || 1;
      var w = Math.max(1, Math.round(canvas.clientWidth * ratio)), h = Math.max(1, Math.round(canvas.clientHeight * ratio));
      if (canvas.width !== w || canvas.height !== h) { canvas.width = w; canvas.height = h; }
      gl.viewport(0, 0, w, h);
      gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
      // models stand on z = 0, z up: turn about z, then lay z up the screen and tilt
      var turn = multiply(rotateX(pitch - Math.PI / 2), rotateZ(yaw));
      var view = new Float32Array([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, -distance, 1]);
      var mvp = multiply(perspective(0.6, w / h, distance / 100, distance * 10), multiply(view, turn));
      gl.uniformMatrix4fv(mvpAt, false, mvp);
      gl.uniformMatrix3fv(turnAt, false, new Float32Array([turn[0], turn[1], turn[2], turn[4], turn[5], turn[6], turn[8], turn[9], turn[10]]));
      gl.drawArrays(gl.TRIANGLES, 0, mesh.count);
    }
    var pending = false;
    function redraw() { if (!pending) { pending = true; requestAnimationFrame(function () { pending = false; draw(); }); } }

    // turning with a finger or the mouse, zooming with the wheel or two fingers
    var pointers = {}, lastSpread = 0;
    function spread() {
      var ids = Object.keys(pointers);
      if (ids.length < 2) return 0;
      var a = pointers[ids[0]], b = pointers[ids[1]];
      return Math.hypot(a.x - b.x, a.y - b.y);
    }
    canvas.addEventListener('pointerdown', function (e) {
      canvas.setPointerCapture(e.pointerId);
      pointers[e.pointerId] = { x: e.clientX, y: e.clientY };
      lastSpread = spread();
    });
    canvas.addEventListener('pointermove', function (e) {
      var p = pointers[e.pointerId];
      if (!p) return;
      if (Object.keys(pointers).length === 1) {
        yaw += (e.clientX - p.x) * 0.01;
        pitch = Math.max(-1.5, Math.min(1.5, pitch + (e.clientY - p.y) * 0.01));
      }
      p.x = e.clientX; p.y = e.clientY;
      var s = spread();
      if (s && lastSpread) distance = Math.max(mesh.radius * 1.2, Math.min(mesh.radius * 12, distance * lastSpread / s));
      lastSpread = s;
      redraw();
    });
    function up(e) { delete pointers[e.pointerId]; lastSpread = spread(); }
    canvas.addEventListener('pointerup', up);
    canvas.addEventListener('pointercancel', up);
    canvas.addEventListener('wheel', function (e) {
      e.preventDefault();
      distance = Math.max(mesh.radius * 1.2, Math.min(mesh.radius * 12, distance * Math.exp(e.deltaY * 0.001)));
      redraw();
    }, { passive: false });
    window.addEventListener('resize', redraw);

    box.hidden = false;
    var picture = document.querySelector('.model-picture');
    if (picture) picture.hidden = true;
    draw();
  }
})();
