/**
 * ScreenshotSafe — Annotation Editor
 *
 * Uses Fabric.js for interactive canvas-based annotation.
 * Reads window.ANNOTATIONS, window.CROP_RECT, and window.SCREENSHOT_ID
 * set by the server-rendered page.
 */

(function () {
    'use strict';

    // ── State ──
    let currentTool = 'select';
    let isDrawing = false;
    let drawStart = null;
    let previewObj = null;
    let undoStack = [];
    let redoStack = [];
    let canvas;
    let backgroundImage;
    let lineHandles = [];

    // ── Initialize ──
    function init() {
        canvas = new fabric.Canvas('editor-canvas', {
            selection: true,
            preserveObjectStacking: true,
            uniformScaling: false,
            uniScaleTransform: false,
            targetFindTolerance: 15,
        });

        // Load the original image as background
        const imgUrl = `/api/screenshots/${window.SCREENSHOT_ID}/original`;
        fabric.Image.fromURL(imgUrl, function (img) {
            backgroundImage = img;
            const wrapper = document.querySelector('.editor-canvas-wrap');
            canvas.setWidth(wrapper.clientWidth);
            canvas.setHeight(wrapper.clientHeight);

            canvas.setBackgroundImage(img, canvas.renderAll.bind(canvas), {
                originX: 'left',
                originY: 'top',
            });

            // Load existing annotations
            loadAnnotations(window.ANNOTATIONS);

            zoomToFit();

            function updateUniformScaling(e) {
                const obj = e.selected ? e.selected[0] : null;
                if (obj && obj.annotationType === 'text') {
                    canvas.uniformScaling = true;
                } else {
                    canvas.uniformScaling = false;
                }
            }
            canvas.on('selection:created', updateUniformScaling);
            canvas.on('selection:updated', updateUniformScaling);
            canvas.on('selection:cleared', function() {
                canvas.uniformScaling = false;
            });

            // Save initial state for undo
            saveUndoState();

            window.addEventListener('resize', function() {
                const wrapper = document.querySelector('.editor-canvas-wrap');
                canvas.setWidth(wrapper.clientWidth);
                canvas.setHeight(wrapper.clientHeight);
                canvas.requestRenderAll();
            });
        }, { crossOrigin: 'anonymous' });

        setupToolbar();
        setupCanvasEvents();
        setupSidebar();
    }

    function zoomToFit() {
        if (!backgroundImage) return;
        const wrapper = document.querySelector('.editor-canvas-wrap');
        const padding = 40;
        const scaleX = Math.max((wrapper.clientWidth - padding) / backgroundImage.width, 0.05);
        const scaleY = Math.max((wrapper.clientHeight - padding) / backgroundImage.height, 0.05);
        let scale = Math.min(scaleX, scaleY, 1); // Don't zoom in past 100% on fit
        
        canvas.setZoom(1);
        canvas.setViewportTransform([
            scale, 0, 0, scale,
            (wrapper.clientWidth - backgroundImage.width * scale) / 2,
            (wrapper.clientHeight - backgroundImage.height * scale) / 2
        ]);
        canvas.requestRenderAll();
    }

    // ── Load annotations from JSON into Fabric objects ──
    function loadAnnotations(annotations) {
        if (!annotations || !Array.isArray(annotations)) return;

        annotations.forEach(function (ann) {
            let obj;
            switch (ann.type) {
                case 'redact':
                    obj = new fabric.Rect({
                        left: ann.x,
                        top: ann.y,
                        width: ann.w,
                        height: ann.h,
                        fill: '#000000',
                        stroke: null,
                        strokeUniform: true,
                        annotationType: 'redact',
                    });
                    obj.setControlsVisibility({ mtr: false });
                    break;
                case 'rect':
                    obj = new fabric.Rect({
                        left: ann.x,
                        top: ann.y,
                        width: ann.w,
                        height: ann.h,
                        fill: ann.filled ? ann.color : 'transparent',
                        stroke: ann.color,
                        strokeWidth: ann.strokeWidth || 3,
                        strokeUniform: true,
                        annotationType: 'rect',
                    });
                    obj.setControlsVisibility({ mtr: false });
                    break;
                case 'arrow':
                    obj = createArrow(ann.x1, ann.y1, ann.x2, ann.y2, ann.color, ann.strokeWidth || 3);
                    obj.hasControls = false;
                    break;
                case 'line':
                    obj = new fabric.Line([ann.x1, ann.y1, ann.x2, ann.y2], {
                        stroke: ann.color,
                        strokeWidth: ann.strokeWidth || 3,
                        annotationType: 'line',
                        _lineData: { x1: ann.x1, y1: ann.y1, x2: ann.x2, y2: ann.y2 },
                        perPixelTargetFind: true
                    });
                    obj._initialLeft = obj.left;
                    obj._initialTop = obj.top;
                    obj.hasControls = false;
                    break;
                case 'text':
                    obj = new fabric.IText(ann.text, {
                        left: ann.x,
                        top: ann.y,
                        fontSize: ann.fontSize || 24,
                        fill: ann.color,
                        fontFamily: 'Arial, sans-serif',
                        annotationType: 'text',
                        lockUniScaling: true,
                    });
                    obj.setControlsVisibility({ mt: false, mb: false, ml: false, mr: false, mtr: false });
                    break;
            }
            if (obj) {
                canvas.add(obj);
            }
        });
        canvas.renderAll();
    }

    // ── Serialize Fabric objects back to annotation JSON ──
    function serializeAnnotations() {
        function getTransformedPoint(obj, localX, localY) {
            let scaleX = obj.scaleX || 1;
            let scaleY = obj.scaleY || 1;
            let lx = (localX - obj._initialLeft) * scaleX;
            let ly = (localY - obj._initialTop) * scaleY;
            let rad = (obj.angle || 0) * Math.PI / 180;
            let rx = lx * Math.cos(rad) - ly * Math.sin(rad);
            let ry = lx * Math.sin(rad) + ly * Math.cos(rad);
            return {
                x: Math.round(obj.left + rx),
                y: Math.round(obj.top + ry)
            };
        }

        function getNormalizedRect(obj) {
            let w = obj.width * (obj.scaleX || 1);
            let h = obj.height * (obj.scaleY || 1);
            let x = obj.left;
            let y = obj.top;
            if (w < 0) { x += w; w = -w; }
            if (h < 0) { y += h; h = -h; }
            return { x: Math.round(x), y: Math.round(y), w: Math.round(w), h: Math.round(h) };
        }

        const annotations = [];
        canvas.getObjects().forEach(function (obj) {
            const type = obj.annotationType;
            if (!type) return;

            switch (type) {
                case 'redact':
                    let r1 = getNormalizedRect(obj);
                    annotations.push({
                        type: 'redact', x: r1.x, y: r1.y, w: r1.w, h: r1.h,
                    });
                    break;
                case 'rect':
                    let r2 = getNormalizedRect(obj);
                    annotations.push({
                        type: 'rect', x: r2.x, y: r2.y, w: r2.w, h: r2.h,
                        color: obj.stroke || obj.fill,
                        filled: obj.fill !== 'transparent',
                        strokeWidth: obj.strokeWidth,
                    });
                    break;
                case 'arrow':
                    if (obj._arrowData) {
                        let p1 = getTransformedPoint(obj, obj._arrowData.x1, obj._arrowData.y1);
                        let p2 = getTransformedPoint(obj, obj._arrowData.x2, obj._arrowData.y2);
                        annotations.push({
                            type: 'arrow',
                            x1: p1.x, y1: p1.y, x2: p2.x, y2: p2.y,
                            color: obj._arrowData.color,
                            strokeWidth: obj._arrowData.strokeWidth,
                        });
                    }
                    break;
                case 'line':
                    if (obj._lineData) {
                        let p1 = getTransformedPoint(obj, obj._lineData.x1, obj._lineData.y1);
                        let p2 = getTransformedPoint(obj, obj._lineData.x2, obj._lineData.y2);
                        annotations.push({
                            type: 'line',
                            x1: p1.x, y1: p1.y, x2: p2.x, y2: p2.y,
                            color: obj.stroke,
                            strokeWidth: obj.strokeWidth,
                        });
                    }
                    break;
                case 'text':
                    annotations.push({
                        type: 'text',
                        x: Math.round(obj.left),
                        y: Math.round(obj.top),
                        text: obj.text,
                        fontSize: Math.round(obj.fontSize * Math.max(Math.abs(obj.scaleX || 1), Math.abs(obj.scaleY || 1))),
                        color: obj.fill,
                    });
                    break;
            }
        });
        return annotations;
    }

    // ── Arrow creation helper ──
    function createArrow(x1, y1, x2, y2, color, strokeWidth) {
        const dx = x2 - x1;
        const dy = y2 - y1;
        const dist = Math.sqrt(dx * dx + dy * dy);
        const angle = Math.atan2(dy, dx);
        const headLen = Math.max(strokeWidth * 5, 15);

        const p0x = x2;
        const p0y = y2;
        const p1x = x2 - headLen * Math.cos(angle - Math.PI / 6);
        const p1y = y2 - headLen * Math.sin(angle - Math.PI / 6);
        const p2x = x2 - (headLen * 0.6) * Math.cos(angle);
        const p2y = y2 - (headLen * 0.6) * Math.sin(angle);
        const p3x = x2 - headLen * Math.cos(angle + Math.PI / 6);
        const p3y = y2 - headLen * Math.sin(angle + Math.PI / 6);

        const lineEndDist = Math.min(headLen * 0.6, dist);
        const lx2 = x2 - lineEndDist * Math.cos(angle);
        const ly2 = y2 - lineEndDist * Math.sin(angle);

        const line = new fabric.Line([x1, y1, lx2, ly2], {
            stroke: color,
            strokeWidth: strokeWidth,
        });

        const head = new fabric.Polygon([
            { x: p0x, y: p0y },
            { x: p1x, y: p1y },
            { x: p2x, y: p2y },
            { x: p3x, y: p3y }
        ], {
            fill: color,
            stroke: color,
            strokeWidth: 1,
            strokeLineJoin: 'miter'
        });

        // Force centers to bypass fabric.js group bounding box shifts when strokeWidth differs
        line.set({
            originX: 'center', originY: 'center',
            left: (x1 + lx2) / 2, top: (y1 + ly2) / 2
        });
        head.set({
            originX: 'center', originY: 'center',
            left: head.left + head.width / 2, top: head.top + head.height / 2
        });

        const group = new fabric.Group([line, head], {
            annotationType: 'arrow',
            _arrowData: { x1, y1, x2, y2, color, strokeWidth },
            perPixelTargetFind: true,
        });
        group._initialLeft = group.left;
        group._initialTop = group.top;

        return group;
    }

    // ── Endpoint Handles ──
    function clearHandles() {
        lineHandles.forEach(h => canvas.remove(h));
        lineHandles = [];
    }

    function getAbsoluteLinePoints(obj) {
        if (obj.annotationType === 'arrow' && obj._arrowData) {
            return {
                p1: { x: obj.left + (obj._arrowData.x1 - obj._initialLeft), y: obj.top + (obj._arrowData.y1 - obj._initialTop) },
                p2: { x: obj.left + (obj._arrowData.x2 - obj._initialLeft), y: obj.top + (obj._arrowData.y2 - obj._initialTop) }
            };
        } else if (obj.annotationType === 'line' && obj._lineData) {
            return {
                p1: { x: obj.left + (obj._lineData.x1 - obj._initialLeft), y: obj.top + (obj._lineData.y1 - obj._initialTop) },
                p2: { x: obj.left + (obj._lineData.x2 - obj._initialLeft), y: obj.top + (obj._lineData.y2 - obj._initialTop) }
            };
        }
        return null;
    }

    function setupHandles(obj) {
        const pts = getAbsoluteLinePoints(obj);
        if (!pts) return;

        const makeHandle = (x, y, isStart) => {
            const h = new fabric.Circle({
                left: x, top: y, radius: 6, fill: '#0088ff',
                stroke: '#ffffff', strokeWidth: 2,
                originX: 'center', originY: 'center',
                hasControls: false, hasBorders: false,
                selectable: true, isHandle: true,
                targetObj: obj, isStart: isStart,
                excludeFromExport: true
            });
            return h;
        };

        const h1 = makeHandle(pts.p1.x, pts.p1.y, true);
        const h2 = makeHandle(pts.p2.x, pts.p2.y, false);

        h1.otherHandle = h2;
        h2.otherHandle = h1;

        lineHandles.push(h1, h2);
        canvas.add(h1, h2);
        h1.bringToFront();
        h2.bringToFront();
    }

    // ── Toolbar setup ──
    function setupToolbar() {
        document.querySelectorAll('.tool-btn[data-tool]').forEach(function (btn) {
            btn.addEventListener('click', function () {
                currentTool = btn.dataset.tool;
                document.querySelectorAll('.tool-btn[data-tool]').forEach(function (b) {
                    b.classList.remove('active');
                });
                btn.classList.add('active');

                // Configure canvas for current tool
                if (currentTool === 'select') {
                    canvas.selection = true;
                    canvas.forEachObject(function (o) { o.selectable = true; });
                } else {
                    canvas.selection = false;
                    canvas.discardActiveObject();
                    canvas.forEachObject(function (o) { o.selectable = false; });
                    canvas.renderAll();
                }
            });
        });

        document.getElementById('stroke-width').addEventListener('input', function(e) {
            const width = parseInt(e.target.value);
            let modified = false;
            canvas.getActiveObjects().forEach(function(obj) {
                if (obj.annotationType === 'rect' || obj.annotationType === 'line' || obj.annotationType === 'arrow') {
                    if (obj.annotationType === 'arrow') {
                        obj._arrowData.strokeWidth = width;
                        obj.getObjects().forEach(function(o) { o.set('strokeWidth', width); });
                    } else {
                        obj.set('strokeWidth', width);
                    }
                    modified = true;
                } else if (obj.annotationType === 'text') {
                    obj.set('fontSize', width * 8);
                    modified = true;
                }
            });
            if (modified) canvas.requestRenderAll();
        });
        document.getElementById('stroke-width').addEventListener('change', function(e) {
            if (canvas.getActiveObjects().length > 0) saveUndoState();
        });

        document.getElementById('annotation-color').addEventListener('input', function(e) {
            const color = e.target.value;
            let modified = false;
            canvas.getActiveObjects().forEach(function(obj) {
                if (obj.annotationType === 'rect') {
                    obj.set('stroke', color);
                    if (obj.fill !== 'transparent' && obj.fill !== '#000000') obj.set('fill', color);
                    modified = true;
                } else if (obj.annotationType === 'line') {
                    obj.set('stroke', color);
                    modified = true;
                } else if (obj.annotationType === 'arrow') {
                    obj._arrowData.color = color;
                    obj.getObjects().forEach(function(o) { o.set('stroke', color); });
                    modified = true;
                } else if (obj.annotationType === 'text') {
                    obj.set('fill', color);
                    modified = true;
                }
            });
            if (modified) canvas.requestRenderAll();
        });
        document.getElementById('annotation-color').addEventListener('change', function(e) {
            if (canvas.getActiveObjects().length > 0) saveUndoState();
        });

        document.getElementById('zoom-in-btn').addEventListener('click', function() {
            let zoom = canvas.getZoom() * 1.2;
            if (zoom > 20) zoom = 20;
            canvas.zoomToPoint({ x: canvas.width / 2, y: canvas.height / 2 }, zoom);
        });

        document.getElementById('zoom-out-btn').addEventListener('click', function() {
            let zoom = canvas.getZoom() / 1.2;
            if (zoom < 0.05) zoom = 0.05;
            canvas.zoomToPoint({ x: canvas.width / 2, y: canvas.height / 2 }, zoom);
        });

        document.getElementById('zoom-fit-btn').addEventListener('click', function() {
            zoomToFit();
        });

        document.getElementById('undo-btn').addEventListener('click', undo);
        document.getElementById('redo-btn').addEventListener('click', redo);
        document.getElementById('reset-btn').addEventListener('click', resetAll);
        document.getElementById('save-btn').addEventListener('click', save);
    }

    // ── Canvas drawing events ──
    function setupCanvasEvents() {
        function onSelection() {
            const activeObjs = canvas.getActiveObjects();
            if (activeObjs.length === 1 && activeObjs[0].isHandle) {
                return;
            }

            clearHandles();

            if (activeObjs.length === 1) {
                let obj = activeObjs[0];
                if (obj.annotationType === 'line' || obj.annotationType === 'arrow') {
                    setupHandles(obj);
                }
            }
        }

        canvas.on('selection:created', onSelection);
        canvas.on('selection:updated', onSelection);
        canvas.on('selection:cleared', onSelection);

        canvas.on('object:moving', function(opt) {
            const obj = opt.target;
            if (obj.isHandle) {
                const isStart = obj.isStart;
                const otherHandle = obj.otherHandle;
                const targetObj = obj.targetObj;
                
                const x1 = isStart ? obj.left : otherHandle.left;
                const y1 = isStart ? obj.top : otherHandle.top;
                const x2 = isStart ? otherHandle.left : obj.left;
                const y2 = isStart ? otherHandle.top : obj.top;

                let newObj;
                if (targetObj.annotationType === 'arrow') {
                    newObj = createArrow(x1, y1, x2, y2, targetObj._arrowData.color, targetObj._arrowData.strokeWidth);
                    newObj.hasControls = false;
                } else {
                    newObj = new fabric.Line([x1, y1, x2, y2], {
                        stroke: targetObj.stroke, strokeWidth: targetObj.strokeWidth,
                        annotationType: 'line',
                        _lineData: { x1, y1, x2, y2 },
                        perPixelTargetFind: true
                    });
                    newObj._initialLeft = newObj.left;
                    newObj._initialTop = newObj.top;
                    newObj.hasControls = false;
                }

                const idx = canvas.getObjects().indexOf(targetObj);
                canvas.remove(targetObj);
                canvas.add(newObj);
                newObj.moveTo(idx);
                
                obj.targetObj = newObj;
                otherHandle.targetObj = newObj;
            } else if (obj.annotationType === 'line' || obj.annotationType === 'arrow') {
                const pts = getAbsoluteLinePoints(obj);
                if (pts && lineHandles.length === 2) {
                    lineHandles.find(h => h.isStart).set({ left: pts.p1.x, top: pts.p1.y }).setCoords();
                    lineHandles.find(h => !h.isStart).set({ left: pts.p2.x, top: pts.p2.y }).setCoords();
                }
            }
        });

        canvas.on('object:modified', function () {
            saveUndoState();
        });

        canvas.on('mouse:wheel', function(opt) {
            let delta = opt.e.deltaY;
            let zoom = canvas.getZoom();
            zoom *= 0.999 ** delta;
            if (zoom > 20) zoom = 20;
            if (zoom < 0.05) zoom = 0.05;
            canvas.zoomToPoint({ x: opt.e.offsetX, y: opt.e.offsetY }, zoom);
            opt.e.preventDefault();
            opt.e.stopPropagation();
        });

        canvas.on('mouse:down', function (opt) {
            const evt = opt.e;
            if (evt.altKey === true || evt.button === 1 || evt.button === 2) {
                canvas.isDragging = true;
                canvas.selection = false;
                canvas.lastPosX = evt.clientX;
                canvas.lastPosY = evt.clientY;
                return;
            }

            if (currentTool === 'select') return;
            isDrawing = true;
            const pointer = canvas.getPointer(opt.e);
            drawStart = { x: pointer.x, y: pointer.y };

            if (currentTool === 'text') {
                const color = document.getElementById('annotation-color').value;
                const fontSize = parseInt(document.getElementById('stroke-width').value) * 8;
                const text = new fabric.IText('Text', {
                    left: pointer.x,
                    top: pointer.y,
                    fontSize: Math.max(fontSize, 16),
                    fill: color,
                    fontFamily: 'Arial, sans-serif',
                    annotationType: 'text',
                    lockUniScaling: true,
                });
                text.setControlsVisibility({ mt: false, mb: false, ml: false, mr: false, mtr: false });
                canvas.add(text);
                canvas.setActiveObject(text);
                text.enterEditing();
                isDrawing = false;
                currentTool = 'select';
                document.querySelector('.tool-btn[data-tool="select"]').click();
                saveUndoState();
            }
        });

        canvas.on('mouse:move', function (opt) {
            if (canvas.isDragging) {
                const e = opt.e;
                const vpt = canvas.viewportTransform;
                vpt[4] += e.clientX - canvas.lastPosX;
                vpt[5] += e.clientY - canvas.lastPosY;
                canvas.requestRenderAll();
                canvas.lastPosX = e.clientX;
                canvas.lastPosY = e.clientY;
                return;
            }

            if (!isDrawing || !drawStart) return;
            
            const pointer = canvas.getPointer(opt.e);
            const x1 = drawStart.x;
            const y1 = drawStart.y;
            const x2 = pointer.x;
            const y2 = pointer.y;
            const color = document.getElementById('annotation-color').value;
            const strokeWidth = parseInt(document.getElementById('stroke-width').value);

            if (previewObj) {
                canvas.remove(previewObj);
                previewObj = null;
            }

            switch (currentTool) {
                case 'redact':
                    previewObj = new fabric.Rect({
                        left: Math.min(x1, x2), top: Math.min(y1, y2),
                        width: Math.abs(x2 - x1), height: Math.abs(y2 - y1),
                        fill: '#000000', stroke: null, strokeUniform: true, selectable: false, evented: false
                    });
                    break;
                case 'rect':
                    previewObj = new fabric.Rect({
                        left: Math.min(x1, x2), top: Math.min(y1, y2),
                        width: Math.abs(x2 - x1), height: Math.abs(y2 - y1),
                        fill: 'transparent', stroke: color, strokeWidth: strokeWidth, strokeUniform: true, selectable: false, evented: false
                    });
                    break;
                case 'arrow':
                    previewObj = createArrow(x1, y1, x2, y2, color, strokeWidth);
                    previewObj.set({ selectable: false, evented: false });
                    break;
                case 'line':
                    previewObj = new fabric.Line([x1, y1, x2, y2], {
                        stroke: color, strokeWidth: strokeWidth, selectable: false, evented: false
                    });
                    break;
                case 'crop':
                    previewObj = new fabric.Rect({
                        left: Math.min(x1, x2), top: Math.min(y1, y2),
                        width: Math.abs(x2 - x1), height: Math.abs(y2 - y1),
                        fill: 'transparent', stroke: '#00ff88', strokeWidth: 2,
                        strokeDashArray: [8, 4], selectable: false, evented: false
                    });
                    break;
            }

            if (previewObj) {
                canvas.add(previewObj);
                canvas.renderAll();
            }
        });

        canvas.on('mouse:up', function (opt) {
            if (canvas.isDragging) {
                canvas.setViewportTransform(canvas.viewportTransform);
                canvas.isDragging = false;
                if (currentTool === 'select') {
                    canvas.selection = true;
                }
                return;
            }

            if (!isDrawing || !drawStart) return;
            isDrawing = false;
            
            if (previewObj) {
                canvas.remove(previewObj);
                previewObj = null;
            }

            const pointer = canvas.getPointer(opt.e);
            const x1 = drawStart.x;
            const y1 = drawStart.y;
            const x2 = pointer.x;
            const y2 = pointer.y;
            const color = document.getElementById('annotation-color').value;
            const strokeWidth = parseInt(document.getElementById('stroke-width').value);

            // Minimum size threshold
            const dx = Math.abs(x2 - x1);
            const dy = Math.abs(y2 - y1);
            if (dx < 3 && dy < 3 && currentTool !== 'text') {
                drawStart = null;
                return;
            }

            let obj;
            switch (currentTool) {
                case 'redact':
                    obj = new fabric.Rect({
                        left: Math.min(x1, x2),
                        top: Math.min(y1, y2),
                        width: dx,
                        height: dy,
                        fill: '#000000',
                        stroke: null,
                        strokeUniform: true,
                        annotationType: 'redact',
                    });
                    obj.setControlsVisibility({ mtr: false });
                    break;
                case 'rect':
                    obj = new fabric.Rect({
                        left: Math.min(x1, x2),
                        top: Math.min(y1, y2),
                        width: dx,
                        height: dy,
                        fill: 'transparent',
                        stroke: color,
                        strokeWidth: strokeWidth,
                        strokeUniform: true,
                        annotationType: 'rect',
                    });
                    obj.setControlsVisibility({ mtr: false });
                    break;
                case 'arrow':
                    obj = createArrow(x1, y1, x2, y2, color, strokeWidth);
                    obj.hasControls = false;
                    break;
                case 'line':
                    obj = new fabric.Line([x1, y1, x2, y2], {
                        stroke: color,
                        strokeWidth: strokeWidth,
                        annotationType: 'line',
                        _lineData: { x1, y1, x2, y2 },
                        perPixelTargetFind: true
                    });
                    obj._initialLeft = obj.left;
                    obj._initialTop = obj.top;
                    obj.hasControls = false;
                    break;
                case 'crop':
                    // Store crop rect (visual feedback only for now)
                    window.CROP_RECT = {
                        x: Math.round(Math.min(x1, x2)),
                        y: Math.round(Math.min(y1, y2)),
                        w: Math.round(dx),
                        h: Math.round(dy),
                    };
                    // Show visual crop indicator
                    showCropIndicator();
                    break;
            }

            if (obj) {
                canvas.add(obj);
                canvas.renderAll();
                saveUndoState();
            }

            drawStart = null;
        });
    }

    function showCropIndicator() {
        // Remove existing crop indicator
        canvas.getObjects().forEach(function (o) {
            if (o._isCropIndicator) canvas.remove(o);
        });

        if (window.CROP_RECT) {
            const rect = new fabric.Rect({
                left: window.CROP_RECT.x,
                top: window.CROP_RECT.y,
                width: window.CROP_RECT.w,
                height: window.CROP_RECT.h,
                fill: 'transparent',
                stroke: '#00ff88',
                strokeWidth: 2,
                strokeDashArray: [8, 4],
                selectable: false,
                evented: false,
                _isCropIndicator: true,
            });
            canvas.add(rect);
            canvas.renderAll();
        }
    }

    // ── Undo / Redo ──
    function saveUndoState() {
        undoStack.push(canvas.toJSON(['annotationType', '_arrowData', '_isCropIndicator']));
        redoStack = [];
        if (undoStack.length > 50) undoStack.shift();
    }

    function undo() {
        if (undoStack.length <= 1) return;
        redoStack.push(undoStack.pop());
        const state = undoStack[undoStack.length - 1];
        canvas.loadFromJSON(state, function () {
            canvas.renderAll();
        });
    }

    function redo() {
        if (redoStack.length === 0) return;
        const state = redoStack.pop();
        undoStack.push(state);
        canvas.loadFromJSON(state, function () {
            canvas.renderAll();
        });
    }

    function resetAll() {
        if (!confirm('Remove all annotations and crop?')) return;
        canvas.getObjects().slice().forEach(function (o) {
            canvas.remove(o);
        });
        window.CROP_RECT = null;
        canvas.renderAll();
        saveUndoState();
    }

    // ── Save ──
    async function save() {
        const saveBtn = document.getElementById('save-btn');
        saveBtn.textContent = 'Saving...';
        saveBtn.disabled = true;

        try {
            // Save annotations
            const annotations = serializeAnnotations();
            const crop = window.CROP_RECT || null;

            const resp = await fetch(`/api/screenshots/${window.SCREENSHOT_ID}/annotations`, {
                method: 'PUT',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ annotations, crop }),
            });

            if (!resp.ok) {
                const data = await resp.json();
                alert('Save failed: ' + (data.error || 'Unknown error'));
                return;
            }

            // Save metadata
            const title = document.getElementById('screenshot-title').value;
            const visibility = document.getElementById('screenshot-visibility').value;

            await fetch(`/api/screenshots/${window.SCREENSHOT_ID}`, {
                method: 'PATCH',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ title, visibility }),
            });

            saveBtn.textContent = 'Saved ✓';
            setTimeout(function () {
                saveBtn.textContent = 'Save';
            }, 2000);
        } catch (err) {
            alert('Save failed: ' + err.message);
        } finally {
            saveBtn.disabled = false;
        }
    }

    // ── Sidebar setup ──
    function setupSidebar() {
        document.getElementById('copy-share-btn').addEventListener('click', function () {
            navigator.clipboard.writeText(document.getElementById('share-url').value);
            this.textContent = 'Copied!';
            setTimeout(() => { this.textContent = 'Copy'; }, 2000);
        });

        document.getElementById('copy-raw-btn').addEventListener('click', function () {
            navigator.clipboard.writeText(document.getElementById('raw-url').value);
            this.textContent = 'Copied!';
            setTimeout(() => { this.textContent = 'Copy'; }, 2000);
        });
    }

    // ── Keyboard shortcuts ──
    document.addEventListener('keydown', function (e) {
        if (e.target.tagName === 'INPUT' || e.target.tagName === 'TEXTAREA') return;

        if ((e.metaKey || e.ctrlKey) && e.key === 'z' && !e.shiftKey) {
            e.preventDefault();
            undo();
        }
        if ((e.metaKey || e.ctrlKey) && (e.key === 'y' || (e.key === 'z' && e.shiftKey))) {
            e.preventDefault();
            redo();
        }
        if ((e.metaKey || e.ctrlKey) && e.key === 's') {
            e.preventDefault();
            save();
        }
        if (e.key === 'Delete' || e.key === 'Backspace') {
            const active = canvas.getActiveObject();
            if (active && !active.isEditing) {
                canvas.remove(active);
                canvas.renderAll();
                saveUndoState();
            }
        }
    });

    // ── Start ──
    if (document.readyState === 'loading') {
        document.addEventListener('DOMContentLoaded', init);
    } else {
        init();
    }
})();
