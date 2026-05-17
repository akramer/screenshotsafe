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
    let selectedAnnotationObjects = [];
    let lastToolbarInteractionAt = 0;
    let autosaveTimer = null;
    let saveInFlight = false;
    let saveAgainAfterCurrent = false;
    let lastSavedSnapshot = null;
    let imageDpi = getImageDpi();
    let visualScale = dpiToVisualScale(imageDpi);
    const AUTOSAVE_DELAY_MS = 5000;

    const dropShadowConfig = {
        color: 'rgba(0,0,0,0.3)',
        blur: 4 * visualScale,
        offsetX: 2 * visualScale,
        offsetY: 2 * visualScale
    };

    function getImageDpi() {
        const dpi = Number(window.IMAGE_DPI);
        return Number.isFinite(dpi) && dpi > 0 ? dpi : 100;
    }

    function dpiToVisualScale(dpi) {
        return Math.max(0.1, Math.min(10, dpi / 100));
    }

    function toImagePixels(value) {
        return value * visualScale;
    }

    function toLogicalPixels(value) {
        return value / visualScale;
    }

    function normalizeDpi(value) {
        const dpi = Number(value);
        if (!Number.isFinite(dpi) || dpi <= 0) return 100;
        return Math.max(1, Math.min(2400, dpi));
    }

    function setEditorDpi(nextDpi) {
        nextDpi = normalizeDpi(nextDpi);
        if (Math.abs(nextDpi - imageDpi) < Number.EPSILON) return;

        const annotations = serializeAnnotations();
        imageDpi = nextDpi;
        visualScale = dpiToVisualScale(imageDpi);
        window.IMAGE_DPI = imageDpi;
        dropShadowConfig.blur = 4 * visualScale;
        dropShadowConfig.offsetX = 2 * visualScale;
        dropShadowConfig.offsetY = 2 * visualScale;

        clearHandles();
        canvas.getObjects().slice().forEach(function (obj) {
            canvas.remove(obj);
        });
        loadAnnotations(annotations);
        showCropIndicator();
        canvas.requestRenderAll();
    }

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
            saveUndoState({ autosave: false });
            lastSavedSnapshot = getSaveSnapshot();

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
                        shadow: dropShadowConfig,
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
                        strokeWidth: toImagePixels(ann.strokeWidth || 3),
                        strokeUniform: true,
                        annotationType: 'rect',
                        shadow: dropShadowConfig,
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
                        strokeWidth: toImagePixels(ann.strokeWidth || 3),
                        annotationType: 'line',
                        _lineData: { x1: ann.x1, y1: ann.y1, x2: ann.x2, y2: ann.y2 },
                        perPixelTargetFind: true,
                        shadow: dropShadowConfig,
                    });
                    obj._initialLeft = obj.left;
                    obj._initialTop = obj.top;
                    obj.hasControls = false;
                    break;
                case 'text':
                    obj = new fabric.IText(ann.text, {
                        left: ann.x,
                        top: ann.y,
                        fontSize: toImagePixels(ann.fontSize || 24),
                        fill: ann.color,
                        fontFamily: 'Arial, sans-serif',
                        annotationType: 'text',
                        lockUniScaling: true,
                        shadow: dropShadowConfig,
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
                        strokeWidth: Math.round(toLogicalPixels(obj.strokeWidth || 3)),
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
                            strokeWidth: Math.round(toLogicalPixels(obj.strokeWidth || 3)),
                        });
                    }
                    break;
                case 'text':
                    annotations.push({
                        type: 'text',
                        x: Math.round(obj.left),
                        y: Math.round(obj.top),
                        text: obj.text,
                        fontSize: Math.round(toLogicalPixels(obj.fontSize * Math.max(Math.abs(obj.scaleX || 1), Math.abs(obj.scaleY || 1)))),
                        color: obj.fill,
                    });
                    break;
            }
        });
        return annotations;
    }

    // ── Arrow creation helper ──
    function createArrow(x1, y1, x2, y2, color, strokeWidth) {
        const renderStrokeWidth = toImagePixels(strokeWidth);
        const dx = x2 - x1;
        const dy = y2 - y1;
        const dist = Math.sqrt(dx * dx + dy * dy);
        const angle = Math.atan2(dy, dx);
        const headLen = Math.max(strokeWidth * 5, 15) * visualScale;

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
            strokeWidth: renderStrokeWidth,
        });

        const head = new fabric.Polygon([
            { x: p0x, y: p0y },
            { x: p1x, y: p1y },
            { x: p2x, y: p2y },
            { x: p3x, y: p3y }
        ], {
            fill: color,
            stroke: color,
            strokeWidth: Math.max(1, visualScale),
            strokeLineJoin: 'miter'
        });

        // Force centers to bypass fabric.js group bounding box shifts when strokeWidth differs
        line.set({
            originX: 'center', originY: 'center',
            left: (x1 + lx2) / 2, top: (y1 + ly2) / 2
        });
        line.setCoords();

        const headMinX = Math.min(p0x, p1x, p2x, p3x);
        const headMaxX = Math.max(p0x, p1x, p2x, p3x);
        const headMinY = Math.min(p0y, p1y, p2y, p3y);
        const headMaxY = Math.max(p0y, p1y, p2y, p3y);

        head.set({
            originX: 'center', originY: 'center',
            left: (headMinX + headMaxX) / 2, top: (headMinY + headMaxY) / 2
        });
        head.setCoords();

        const group = new fabric.Group([line, head], {
            annotationType: 'arrow',
            _arrowData: { x1, y1, x2, y2, color, strokeWidth },
            perPixelTargetFind: true,
            shadow: dropShadowConfig,
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

    function isEditableAnnotation(obj) {
        return obj && (
            obj.annotationType === 'rect' ||
            obj.annotationType === 'line' ||
            obj.annotationType === 'arrow' ||
            obj.annotationType === 'text'
        );
    }

    function getEditableSelection() {
        const activeObjects = canvas.getActiveObjects().filter(function(obj) {
            return isEditableAnnotation(obj);
        });

        if (activeObjects.length > 0) {
            selectedAnnotationObjects = activeObjects;
            return activeObjects;
        }

        selectedAnnotationObjects = selectedAnnotationObjects.filter(function(obj) {
            return obj && canvas.contains(obj) && isEditableAnnotation(obj);
        });
        return selectedAnnotationObjects;
    }

    function getLogicalStrokeControlValue(obj) {
        if (!obj) return null;
        if (obj.annotationType === 'arrow' && obj._arrowData) {
            return obj._arrowData.strokeWidth;
        }
        if (obj.annotationType === 'text') {
            return Math.round(toLogicalPixels(obj.fontSize || 24) / 8);
        }
        if (obj.annotationType === 'rect' || obj.annotationType === 'line') {
            return Math.round(toLogicalPixels(obj.strokeWidth || toImagePixels(3)));
        }
        return null;
    }

    function syncToolbarToSelection() {
        const selected = getEditableSelection();
        if (selected.length !== 1) return;

        const strokeWidthInput = document.getElementById('stroke-width');
        const width = getLogicalStrokeControlValue(selected[0]);
        if (strokeWidthInput && width !== null) {
            strokeWidthInput.value = Math.max(
                Number(strokeWidthInput.min),
                Math.min(Number(strokeWidthInput.max), width)
            );
        }

        const colorInput = document.getElementById('annotation-color');
        if (!colorInput) return;
        const obj = selected[0];
        const color = obj.annotationType === 'arrow' && obj._arrowData
            ? obj._arrowData.color
            : (obj.stroke || obj.fill);
        if (typeof color === 'string' && /^#[0-9a-f]{6}$/i.test(color)) {
            colorInput.value = color;
        }
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
        document.getElementById('toolbar').addEventListener('pointerdown', function() {
            selectedAnnotationObjects = getEditableSelection();
            lastToolbarInteractionAt = Date.now();
        }, true);

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

        function applyStrokeWidthToObject(obj, width) {
            if (!obj) return null;

            if (obj.annotationType === 'arrow' && obj._arrowData) {
                if (obj.group && obj.group.type === 'activeSelection') {
                    obj._arrowData.strokeWidth = width;
                    obj.getObjects().forEach(function(child) {
                        if (child.type === 'line') {
                            child.set('strokeWidth', toImagePixels(width));
                        }
                        child.dirty = true;
                    });
                    obj.dirty = true;
                    obj.setCoords();
                    return obj;
                }

                const pts = getAbsoluteLinePoints(obj);
                if (!pts) return obj;

                const replacement = createArrow(
                    pts.p1.x,
                    pts.p1.y,
                    pts.p2.x,
                    pts.p2.y,
                    obj._arrowData.color,
                    width
                );
                replacement.hasControls = false;

                const idx = canvas.getObjects().indexOf(obj);
                canvas.remove(obj);
                canvas.add(replacement);
                replacement.moveTo(idx);
                return replacement;
            }

            if (obj.annotationType === 'rect' || obj.annotationType === 'line') {
                obj.set('strokeWidth', toImagePixels(width));
                obj.dirty = true;
                obj.setCoords();
                return obj;
            }

            if (obj.annotationType === 'text') {
                obj.set('fontSize', toImagePixels(width * 8));
                obj.dirty = true;
                obj.setCoords();
                return obj;
            }

            return obj;
        }

        document.getElementById('stroke-width').addEventListener('input', function(e) {
            const width = parseInt(e.target.value);
            const activeObjects = getEditableSelection();
            const activeObject = canvas.getActiveObject();
            let modified = false;

            clearHandles();
            if (activeObject && activeObject.type === 'activeSelection') {
                canvas.discardActiveObject();
            }

            selectedAnnotationObjects = activeObjects.map(function(obj) {
                const updated = applyStrokeWidthToObject(obj, width);
                if (updated) modified = true;
                return updated;
            }).filter(Boolean);

            if (modified) {
                if (selectedAnnotationObjects.length === 1) {
                    canvas.setActiveObject(selectedAnnotationObjects[0]);
                    if (selectedAnnotationObjects[0].annotationType === 'line' || selectedAnnotationObjects[0].annotationType === 'arrow') {
                        setupHandles(selectedAnnotationObjects[0]);
                    }
                } else if (selectedAnnotationObjects.length > 1) {
                    const selection = new fabric.ActiveSelection(selectedAnnotationObjects, { canvas: canvas });
                    canvas.setActiveObject(selection);
                }
                canvas.requestRenderAll();
                scheduleAutosave();
            }
        });
        document.getElementById('stroke-width').addEventListener('change', function() {
            if (getEditableSelection().length > 0) saveUndoState();
        });

        function applyColorToObject(obj, color) {
            if (obj.annotationType === 'rect') {
                obj.set('stroke', color);
                if (obj.fill !== 'transparent' && obj.fill !== '#000000') obj.set('fill', color);
                obj.dirty = true;
                return true;
            }
            if (obj.annotationType === 'line') {
                obj.set('stroke', color);
                obj.dirty = true;
                return true;
            }
            if (obj.annotationType === 'arrow') {
                obj._arrowData.color = color;
                obj.getObjects().forEach(function(o) {
                    o.set('stroke', color);
                    if (o.type === 'polygon') o.set('fill', color);
                    o.dirty = true;
                });
                obj.dirty = true;
                return true;
            }
            if (obj.annotationType === 'text') {
                obj.set('fill', color);
                obj.dirty = true;
                return true;
            }
            return false;
        }

        document.getElementById('annotation-color').addEventListener('input', function(e) {
            const color = e.target.value;
            let modified = false;
            getEditableSelection().forEach(function(obj) {
                if (applyColorToObject(obj, color)) modified = true;
            });
            if (modified) canvas.requestRenderAll();
        });
        document.getElementById('annotation-color').addEventListener('change', function() {
            if (getEditableSelection().length > 0) saveUndoState();
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
        const legacySaveBtn = document.getElementById('save-btn');
        if (legacySaveBtn) legacySaveBtn.addEventListener('click', flushAutosave);
    }

    // ── Canvas drawing events ──
    function setupCanvasEvents() {
        function onSelection() {
            const activeObjs = canvas.getActiveObjects();
            if (activeObjs.length === 1 && activeObjs[0].isHandle) {
                return;
            }

            clearHandles();
            if (activeObjs.length > 0 || Date.now() - lastToolbarInteractionAt > 250) {
                selectedAnnotationObjects = activeObjs.filter(function(obj) {
                    return isEditableAnnotation(obj);
                });
            }
            syncToolbarToSelection();

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
                        perPixelTargetFind: true,
                        shadow: dropShadowConfig,
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

        canvas.on('text:changed', function () {
            scheduleAutosave();
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
                    fontSize: toImagePixels(Math.max(fontSize, 16)),
                    fill: color,
                    fontFamily: 'Arial, sans-serif',
                    annotationType: 'text',
                    lockUniScaling: true,
                    shadow: dropShadowConfig,
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
                        fill: '#000000', stroke: null, strokeUniform: true, selectable: false, evented: false,
                        shadow: dropShadowConfig,
                    });
                    break;
                case 'rect':
                    previewObj = new fabric.Rect({
                        left: Math.min(x1, x2), top: Math.min(y1, y2),
                        width: Math.abs(x2 - x1), height: Math.abs(y2 - y1),
                        fill: 'transparent', stroke: color, strokeWidth: toImagePixels(strokeWidth), strokeUniform: true, selectable: false, evented: false,
                        shadow: dropShadowConfig,
                    });
                    break;
                case 'arrow':
                    previewObj = createArrow(x1, y1, x2, y2, color, strokeWidth);
                    previewObj.set({ selectable: false, evented: false });
                    break;
                case 'line':
                    previewObj = new fabric.Line([x1, y1, x2, y2], {
                        stroke: color, strokeWidth: toImagePixels(strokeWidth), selectable: false, evented: false,
                        shadow: dropShadowConfig,
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
                        shadow: dropShadowConfig,
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
                        strokeWidth: toImagePixels(strokeWidth),
                        strokeUniform: true,
                        annotationType: 'rect',
                        shadow: dropShadowConfig,
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
                        strokeWidth: toImagePixels(strokeWidth),
                        annotationType: 'line',
                        _lineData: { x1, y1, x2, y2 },
                        perPixelTargetFind: true,
                        shadow: dropShadowConfig,
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
    function cloneCropRect() {
        return window.CROP_RECT ? { ...window.CROP_RECT } : null;
    }

    function getEditorState() {
        return {
            canvas: canvas.toJSON(['annotationType', '_arrowData', '_lineData', '_initialLeft', '_initialTop', '_isCropIndicator']),
            crop: cloneCropRect(),
        };
    }

    function saveUndoState(options = {}) {
        const shouldAutosave = options.autosave !== false;
        undoStack.push(getEditorState());
        redoStack = [];
        if (undoStack.length > 50) undoStack.shift();
        if (shouldAutosave) scheduleAutosave();
    }

    function restoreEditorState(state) {
        window.CROP_RECT = state.crop ? { ...state.crop } : null;
        canvas.loadFromJSON(state.canvas, function () {
            canvas.renderAll();
            scheduleAutosave();
        });
    }

    function undo() {
        if (undoStack.length <= 1) return;
        redoStack.push(undoStack.pop());
        const state = undoStack[undoStack.length - 1];
        restoreEditorState(state);
    }

    function redo() {
        if (redoStack.length === 0) return;
        const state = redoStack.pop();
        undoStack.push(state);
        restoreEditorState(state);
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

    // ── Autosave ──
    function setSaveStatus(text, statusClass) {
        const status = document.getElementById('save-status');
        if (!status) return;
        status.textContent = text;
        status.classList.remove('is-saving', 'is-error');
        if (statusClass) status.classList.add(statusClass);
    }

    function getSavePayload() {
        const title = document.getElementById('screenshot-title').value;
        const sourceUrl = document.getElementById('screenshot-source-url').value;
        const visibility = document.getElementById('screenshot-visibility').value;
        const expiresIn = document.getElementById('screenshot-expires-in').value;
        const metadata = {
            title,
            source_url: sourceUrl,
            visibility,
            image_dpi: normalizeDpi(document.getElementById('screenshot-image-dpi').value),
        };
        if (expiresIn) {
            metadata.expires_in = expiresIn;
        }

        return {
            annotations: serializeAnnotations(),
            crop: window.CROP_RECT || null,
            metadata,
        };
    }

    function getSaveSnapshot() {
        return JSON.stringify(getSavePayload());
    }

    function isSafeSourceUrl(url) {
        const trimmed = url.trim();
        return trimmed.startsWith('http://') || trimmed.startsWith('https://');
    }

    function updateSourceUrlLink() {
        const input = document.getElementById('screenshot-source-url');
        const link = document.getElementById('source-url-link');
        if (!input || !link) return;

        const url = input.value.trim();
        if (isSafeSourceUrl(url)) {
            link.href = url;
            link.hidden = false;
        } else {
            link.removeAttribute('href');
            link.hidden = true;
        }
    }

    function scheduleAutosave(delay = AUTOSAVE_DELAY_MS) {
        if (!canvas) return;
        setSaveStatus('Saving...', 'is-saving');
        clearTimeout(autosaveTimer);
        autosaveTimer = setTimeout(save, delay);
    }

    async function flushAutosave() {
        clearTimeout(autosaveTimer);
        await save();
    }

    function sendKeepaliveRequest(url, method, body) {
        try {
            fetch(url, {
                method,
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify(body),
                keepalive: true,
            });
        } catch (err) {
            // The page is closing, so there is no useful recovery path here.
        }
    }

    function flushAutosaveOnPageExit() {
        clearTimeout(autosaveTimer);
        const snapshot = getSaveSnapshot();
        if (snapshot === lastSavedSnapshot) return;

        const payload = JSON.parse(snapshot);
        sendKeepaliveRequest(`/api/screenshots/${window.SCREENSHOT_ID}/annotations`, 'PUT', {
            annotations: payload.annotations,
            crop: payload.crop,
        });
        sendKeepaliveRequest(`/api/screenshots/${window.SCREENSHOT_ID}`, 'PATCH', payload.metadata);
        lastSavedSnapshot = snapshot;
    }

    async function save() {
        const snapshot = getSaveSnapshot();
        if (snapshot === lastSavedSnapshot && !saveAgainAfterCurrent) {
            setSaveStatus('Saved');
            return;
        }

        if (saveInFlight) {
            saveAgainAfterCurrent = true;
            return;
        }

        try {
            saveInFlight = true;
            saveAgainAfterCurrent = false;
            setSaveStatus('Saving...', 'is-saving');
            const payload = JSON.parse(snapshot);

            const resp = await fetch(`/api/screenshots/${window.SCREENSHOT_ID}/annotations`, {
                method: 'PUT',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ annotations: payload.annotations, crop: payload.crop }),
            });

            if (!resp.ok) {
                const data = await resp.json();
                setSaveStatus(data.error || 'Save failed', 'is-error');
                return;
            }

            const metadataResp = await fetch(`/api/screenshots/${window.SCREENSHOT_ID}`, {
                method: 'PATCH',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify(payload.metadata),
            });

            if (!metadataResp.ok) {
                setSaveStatus('Save failed', 'is-error');
                return;
            }

            lastSavedSnapshot = snapshot;
            setSaveStatus('Saved');
        } catch (err) {
            setSaveStatus('Save failed', 'is-error');
        } finally {
            saveInFlight = false;
            if (saveAgainAfterCurrent) {
                saveAgainAfterCurrent = false;
                scheduleAutosave();
            }
        }
    }

    // ── Sidebar setup ──
    function setupSidebar() {
        document.getElementById('screenshot-title').addEventListener('input', function () {
            scheduleAutosave();
        });
        document.getElementById('screenshot-source-url').addEventListener('input', function () {
            updateSourceUrlLink();
            scheduleAutosave();
        });
        document.getElementById('screenshot-image-dpi').addEventListener('change', function () {
            const dpi = normalizeDpi(this.value);
            this.value = Math.round(dpi);
            setEditorDpi(dpi);
            scheduleAutosave();
        });
        document.getElementById('screenshot-visibility').addEventListener('change', function () {
            scheduleAutosave();
        });
        document.getElementById('screenshot-expires-in').addEventListener('change', function () {
            scheduleAutosave();
        });

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

        updateSourceUrlLink();
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
            flushAutosave();
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

    window.addEventListener('pagehide', flushAutosaveOnPageExit);
    window.addEventListener('beforeunload', flushAutosaveOnPageExit);

    // ── Start ──
    if (document.readyState === 'loading') {
        document.addEventListener('DOMContentLoaded', init);
    } else {
        init();
    }
})();
