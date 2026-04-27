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

    // ── Initialize ──
    function init() {
        canvas = new fabric.Canvas('editor-canvas', {
            selection: true,
            preserveObjectStacking: true,
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
        canvas.viewportTransform = [
            scale, 0, 0, scale,
            (wrapper.clientWidth - backgroundImage.width * scale) / 2,
            (wrapper.clientHeight - backgroundImage.height * scale) / 2
        ];
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
                        annotationType: 'redact',
                    });
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
                        annotationType: 'rect',
                    });
                    break;
                case 'arrow':
                    obj = createArrow(ann.x1, ann.y1, ann.x2, ann.y2, ann.color, ann.strokeWidth || 3);
                    break;
                case 'line':
                    obj = new fabric.Line([ann.x1, ann.y1, ann.x2, ann.y2], {
                        stroke: ann.color,
                        strokeWidth: ann.strokeWidth || 3,
                        annotationType: 'line',
                    });
                    break;
                case 'text':
                    obj = new fabric.IText(ann.text, {
                        left: ann.x,
                        top: ann.y,
                        fontSize: ann.fontSize || 24,
                        fill: ann.color,
                        fontFamily: 'Arial, sans-serif',
                        annotationType: 'text',
                    });
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
        const annotations = [];
        canvas.getObjects().forEach(function (obj) {
            const type = obj.annotationType;
            if (!type) return;

            switch (type) {
                case 'redact':
                    annotations.push({
                        type: 'redact',
                        x: Math.round(obj.left),
                        y: Math.round(obj.top),
                        w: Math.round(obj.width * obj.scaleX),
                        h: Math.round(obj.height * obj.scaleY),
                    });
                    break;
                case 'rect':
                    annotations.push({
                        type: 'rect',
                        x: Math.round(obj.left),
                        y: Math.round(obj.top),
                        w: Math.round(obj.width * obj.scaleX),
                        h: Math.round(obj.height * obj.scaleY),
                        color: obj.stroke || obj.fill,
                        filled: obj.fill !== 'transparent',
                        strokeWidth: obj.strokeWidth,
                    });
                    break;
                case 'arrow':
                    if (obj._arrowData) {
                        annotations.push({
                            type: 'arrow',
                            x1: Math.round(obj._arrowData.x1),
                            y1: Math.round(obj._arrowData.y1),
                            x2: Math.round(obj._arrowData.x2),
                            y2: Math.round(obj._arrowData.y2),
                            color: obj._arrowData.color,
                            strokeWidth: obj._arrowData.strokeWidth,
                        });
                    }
                    break;
                case 'line':
                    annotations.push({
                        type: 'line',
                        x1: Math.round(obj.x1 + obj.left),
                        y1: Math.round(obj.y1 + obj.top),
                        x2: Math.round(obj.x2 + obj.left),
                        y2: Math.round(obj.y2 + obj.top),
                        color: obj.stroke,
                        strokeWidth: obj.strokeWidth,
                    });
                    break;
                case 'text':
                    annotations.push({
                        type: 'text',
                        x: Math.round(obj.left),
                        y: Math.round(obj.top),
                        text: obj.text,
                        fontSize: obj.fontSize,
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
        const angle = Math.atan2(dy, dx);
        const headLen = Math.max(strokeWidth * 5, 15);

        const line = new fabric.Line([x1, y1, x2, y2], {
            stroke: color,
            strokeWidth: strokeWidth,
        });

        const head1 = new fabric.Line([
            x2, y2,
            x2 - headLen * Math.cos(angle - Math.PI / 6),
            y2 - headLen * Math.sin(angle - Math.PI / 6),
        ], { stroke: color, strokeWidth: strokeWidth });

        const head2 = new fabric.Line([
            x2, y2,
            x2 - headLen * Math.cos(angle + Math.PI / 6),
            y2 - headLen * Math.sin(angle + Math.PI / 6),
        ], { stroke: color, strokeWidth: strokeWidth });

        const group = new fabric.Group([line, head1, head2], {
            annotationType: 'arrow',
            _arrowData: { x1, y1, x2, y2, color, strokeWidth },
        });

        return group;
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
                });
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
                        fill: '#000000', stroke: null, selectable: false, evented: false
                    });
                    break;
                case 'rect':
                    previewObj = new fabric.Rect({
                        left: Math.min(x1, x2), top: Math.min(y1, y2),
                        width: Math.abs(x2 - x1), height: Math.abs(y2 - y1),
                        fill: 'transparent', stroke: color, strokeWidth: strokeWidth, selectable: false, evented: false
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
                        annotationType: 'redact',
                    });
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
                        annotationType: 'rect',
                    });
                    break;
                case 'arrow':
                    obj = createArrow(x1, y1, x2, y2, color, strokeWidth);
                    break;
                case 'line':
                    obj = new fabric.Line([x1, y1, x2, y2], {
                        stroke: color,
                        strokeWidth: strokeWidth,
                        annotationType: 'line',
                    });
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
