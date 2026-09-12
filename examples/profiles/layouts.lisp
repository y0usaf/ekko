;; Public tiled and floating workspace policies. This file can be loaded by
;; user configuration in either the regular or bare runtime. Registration is
;; explicit so loading helper definitions alone does not mount host effects.
(defpackage #:ekko/layout-providers
  (:use #:cl #:ekko/extensions)
  (:export #:install-layouts #:tiled-layout #:floating-layout))
(in-package #:ekko/layout-providers)

(defun fit-insets (width height insets)
  (let* ((top (min (first insets) (max 0 (1- height))))
         (left (min (fourth insets) (max 0 (1- width))))
         (bottom (min (third insets) (max 0 (- height top 1))))
         (right (min (second insets) (max 0 (- width left 1)))))
    (list top right bottom left)))

(defun layout-viewport (snapshot)
  (let* ((viewport (value snapshot :viewport)) (geometry (value snapshot :geometry))
         (cols (getf viewport :cols)) (rows (getf viewport :rows))
         (insets (fit-insets cols rows (getf geometry :viewport-insets '(0 0 1 0)))))
    (values (fourth insets) (first insets)
            (- cols (second insets) (fourth insets))
            (- rows (first insets) (third insets)))))

(defun frame-placement (snapshot id x y width height)
  (let* ((geometry (value snapshot :geometry))
         (requested (copy-list (getf geometry :pane-insets '(1 0 0 0))))
         (boundary (getf geometry :boundary-insets)))
    (when boundary
      (multiple-value-bind (left top vw vh) (layout-viewport snapshot)
        (loop for outside in (list (= y top) (= (+ x width) (+ left vw))
                                   (= (+ y height) (+ top vh)) (= x left))
              for index from 0 when outside do (setf (nth index requested) (nth index boundary)))))
    (let ((insets (fit-insets width height requested)))
      (list :pane id :x (+ x (fourth insets)) :y (+ y (first insets))
            :cols (- width (second insets) (fourth insets))
            :rows (- height (first insets) (third insets))
            :outer (list x y width height)))))

(defun provider-tree (tree panes)
  "Keep only eligible pane IDs and add panes from other sessions in scope."
  (labels ((prune (node)
             (cond ((null node) nil)
                   ((integerp node) (when (find node panes :key (lambda (pane) (getf pane :id))) node))
                   (t (let ((a (prune (third node))) (b (prune (fourth node))))
                        (cond ((null a) b) ((null b) a)
                              (t (list (first node) (second node) a b)))))))
           (contains (node id)
             (if (integerp node) (= node id)
                 (and node (or (contains (third node) id) (contains (fourth node) id))))))
    (let ((result (prune tree)))
      (dolist (pane panes result)
        (let ((id (getf pane :id)))
          (unless (contains result id)
            (setf result (if result (list :columns 50 result id) id))))))))

(defun provider-minimums (tree pane boundary)
  (labels ((walk (node edges)
             (if (integerp node)
                 (let ((insets (loop for inside in pane for outside in (or boundary pane)
                                     for edge in edges collect (if edge outside inside))))
                   (list (cons node (list (+ 1 (second insets) (fourth insets))
                                         (+ 1 (first insets) (third insets))))))
                 (let ((a (copy-list edges)) (b (copy-list edges)))
                   (if (eq (first node) :columns)
                       (setf (second a) nil (fourth b) nil)
                       (setf (third a) nil (first b) nil))
                   (append (walk (third node) a) (walk (fourth node) b))))))
    (walk tree '(t t t t))))

(defun floating-rectangle (pane index left top width height)
  (destructuring-bind (x y w h)
      (or (getf pane :floating)
          (list (+ left (* 3 (mod index 8))) (+ top (mod index 8))
                (max 12 (floor (* width 3) 4)) (max 4 (floor (* height 3) 4))))
    (let ((w (min width (max 12 w))) (h (min height (max 4 h))))
      (list (max left (min x (+ left width (- w))))
            (max top (min y (+ top height (- h)))) w h))))

(defun layout-actions (snapshot placements)
  ;; Zoom is provider policy. Hidden panes retain this provider's base demand,
  ;; not dimensions reconstructed by a privileged host split-tree algorithm.
  (let ((focus (value snapshot :focus)))
    (when (and (value snapshot :zoom)
               (find focus placements :key (lambda (placement) (getf placement :pane))))
      (multiple-value-bind (left top width height) (layout-viewport snapshot)
        (setf placements
              (append
                (loop for placement in placements unless (eql focus (getf placement :pane))
                      collect (let ((hidden (copy-list placement)))
                                (setf (getf hidden :visible) nil)
                                hidden))
                (list (frame-placement snapshot focus left top width height)))))))
  (list (action :place-panes :version 1 :placements placements :camera '(0 0))))

(defun tiled-layout (snapshot event)
  (declare (ignore event))
  (let* ((panes (remove-if (lambda (pane) (getf pane :minimized)) (value snapshot :panes)))
         (focus (value snapshot :focus))
         (tiled (remove-if (lambda (pane) (getf pane :floating)) panes))
         (tree (provider-tree (value snapshot :layout) tiled))
         (geometry (value snapshot :geometry))
         (gaps (getf geometry :split-gaps '(1 0)))
         (minimums (when tree (provider-minimums tree (getf geometry :pane-insets '(1 0 0 0))
                                                  (getf geometry :boundary-insets)))))
    (multiple-value-bind (left top width height) (layout-viewport snapshot)
      (let ((rectangles
              (append
                (when tree
                  (mapcar (lambda (rect)
                            (list (first rect) (+ left (second rect)) (+ top (third rect))
                                  (fourth rect) (fifth rect)))
                          (ekko/layout:rectangles
                            tree width height
                            (if (find focus tiled :key (lambda (pane) (getf pane :id)))
                                focus (getf (first tiled) :id)) nil
                            :leaf-min (lambda (id) (cdr (assoc id minimums)))
                            :column-gap (first gaps) :row-gap (second gaps))))
                (loop for pane in (sort (copy-list panes) #'<
                                       :key (lambda (pane) (getf pane :activation-order 0)))
                      for index from 0 when (getf pane :floating)
                        collect (cons (getf pane :id)
                                      (floating-rectangle pane index left top width height))))))
        (layout-actions snapshot
                        (mapcar (lambda (rect) (apply #'frame-placement snapshot rect)) rectangles))))))

(defun floating-layout (snapshot event)
  (declare (ignore event))
  (multiple-value-bind (left top width height) (layout-viewport snapshot)
    (layout-actions snapshot
      (loop for pane in (sort (copy-list (value snapshot :panes)) #'<
                             :key (lambda (pane) (getf pane :activation-order 0)))
            for index from 0 unless (getf pane :minimized)
              collect (apply #'frame-placement snapshot (getf pane :id)
                             (floating-rectangle pane index left top width height))))))

(defun install-layouts ()
  (register-component :id :layouts)
  (dolist (provider (list (cons "tiled" #'tiled-layout) (cons "floating" #'floating-layout)))
    (register-layout-provider :component :layouts :name (car provider)
                              :reads '(:panes :focus :viewport :geometry :layout :zoom)
                              :handler (cdr provider)))
  (set-option :component :layouts :name :layout-provider :value "tiled")
  (set-option :component :layouts :name :workspace-scope :value :session)
  (set-option :component :layouts :name :pane-budget :value 16))
