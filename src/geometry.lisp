(defpackage #:ekko/scene
  (:use #:cl)
  (:export #:rect #:make-rect #:rect-x #:rect-y #:rect-width #:rect-height
           #:rect-right #:rect-bottom #:rect-area
           #:rect-intersection #:rect-subtract #:mapped-fragment
           #:make-mapped-fragment #:mapped-fragment-destination
           #:mapped-fragment-source #:clip-placement))

(in-package #:ekko/scene)

(defconstant +coordinate-limit+ 1000000000)
(defconstant +fragment-limit+ 4096)

(defstruct (rect (:constructor %make-rect (x y width height)))
  (x 0 :type rational :read-only t)
  (y 0 :type rational :read-only t)
  (width 0 :type rational :read-only t)
  (height 0 :type rational :read-only t))

(defun coordinate-p (value)
  (and (rationalp value) (<= (abs value) +coordinate-limit+)))

(defun make-rect (x y width height)
  (unless (and (coordinate-p x) (coordinate-p y)
               (coordinate-p width) (coordinate-p height)
               (plusp width) (plusp height)
               (<= (+ x width) +coordinate-limit+)
               (<= (+ y height) +coordinate-limit+))
    (error "Invalid rectangle (~S ~S ~S ~S)" x y width height))
  (%make-rect x y width height))

(defun rect-right (rect) (+ (rect-x rect) (rect-width rect)))
(defun rect-bottom (rect) (+ (rect-y rect) (rect-height rect)))
(defun rect-area (rect) (* (rect-width rect) (rect-height rect)))
(defun require-rect (value)
  (unless (typep value 'rect) (error "Expected a rectangle: ~S" value))
  value)

(defun rect-intersection (left right)
  (require-rect left) (require-rect right)
  (let ((x (max (rect-x left) (rect-x right)))
        (y (max (rect-y left) (rect-y right)))
        (right-edge (min (rect-right left) (rect-right right)))
        (bottom-edge (min (rect-bottom left) (rect-bottom right))))
    (when (and (< x right-edge) (< y bottom-edge))
      (make-rect x y (- right-edge x) (- bottom-edge y)))))

(defun rect-subtract (subject occluder)
  "Return disjoint half-open pieces of SUBJECT not covered by OCCLUDER."
  (require-rect subject) (require-rect occluder)
  (let ((cut (rect-intersection subject occluder)))
    (if (null cut)
        (list subject)
        (remove nil
                (list
                 (and (< (rect-y subject) (rect-y cut))
                      (make-rect (rect-x subject) (rect-y subject)
                                 (rect-width subject) (- (rect-y cut) (rect-y subject))))
                 (and (< (rect-bottom cut) (rect-bottom subject))
                      (make-rect (rect-x subject) (rect-bottom cut)
                                 (rect-width subject) (- (rect-bottom subject) (rect-bottom cut))))
                 (and (< (rect-x subject) (rect-x cut))
                      (make-rect (rect-x subject) (rect-y cut)
                                 (- (rect-x cut) (rect-x subject)) (rect-height cut)))
                 (and (< (rect-right cut) (rect-right subject))
                      (make-rect (rect-right cut) (rect-y cut)
                                 (- (rect-right subject) (rect-right cut)) (rect-height cut))))))))

(defstruct (mapped-fragment (:constructor %make-mapped-fragment (destination source)))
  (destination nil :read-only t) (source nil :read-only t))

(defun make-mapped-fragment (destination source)
  (require-rect destination) (require-rect source)
  (%make-mapped-fragment destination source))

(defun map-fragment (fragment destination source)
  (let ((scale-x (/ (rect-width source) (rect-width destination)))
        (scale-y (/ (rect-height source) (rect-height destination))))
    (make-mapped-fragment
     fragment
     (make-rect (+ (rect-x source) (* (- (rect-x fragment) (rect-x destination)) scale-x))
                (+ (rect-y source) (* (- (rect-y fragment) (rect-y destination)) scale-y))
                (* (rect-width fragment) scale-x)
                (* (rect-height fragment) scale-y)))))

(defun clip-placement (destination source pane client visible &optional overlays
                                                     &key (max-fragments +fragment-limit+))
  "Clip DESTINATION against bounds and opaque OVERLAYS, mapping SOURCE exactly."
  (require-rect destination) (require-rect source) (require-rect pane)
  (require-rect client) (require-rect visible)
  (unless (and (integerp max-fragments) (plusp max-fragments)
               (<= max-fragments +fragment-limit+))
    (error "Invalid fragment limit: ~S" max-fragments))
  (unless (and (listp overlays) (<= (length overlays) +fragment-limit+))
    (error "Invalid overlay list or overlay limit"))
  (let ((region (rect-intersection destination pane)))
    (setf region (and region (rect-intersection region client)))
    (setf region (and region (rect-intersection region visible)))
    (let ((pieces (if region (list region) nil)))
      (dolist (overlay overlays)
        (require-rect overlay)
        (setf pieces (mapcan (lambda (piece) (rect-subtract piece overlay)) pieces))
        (when (> (length pieces) max-fragments)
          (error "Placement clipping exceeded fragment limit")))
      (mapcar (lambda (piece) (map-fragment piece destination source)) pieces))))
