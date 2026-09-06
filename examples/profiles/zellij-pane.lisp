;; Pure Pane-mode policy for the pinned Zellij 0.43.1 bindings.
;;
;; Directional focus follows the tiled-pane rule: the candidate must share an
;; edge with the focused pane and overlap it on the perpendicular axis.  A
;; daemon supplied activation order breaks ties exactly as Zellij's active_at
;; timestamp does.  The helper only consumes the detached snapshot.
(in-package #:cl-user)

(defun zellij-pane-rect (pane)
  ;; during fullscreen mode.
  (let ((rect (getf pane :layout-rect)))
    (when (and (listp rect) (= (length rect) 4)
               (every #'integerp rect)
               (<= 0 (first rect)) (<= 0 (second rect))
               (plusp (third rect)) (plusp (fourth rect)))
      rect)))

(defun zellij-pane-overlaps-p (a-start a-size b-start b-size)
  (and (< a-start (+ b-start b-size))
       (< b-start (+ a-start a-size))))

(defun zellij-pane-adjacent-p (candidate current direction)
  (destructuring-bind (cx cy cw ch) (zellij-pane-rect candidate)
    (destructuring-bind (x y w h) (zellij-pane-rect current)
      (case direction
        (:left (and (= (+ cx cw) x) (zellij-pane-overlaps-p cy ch y h)))
        (:right (and (= (+ x w) cx) (zellij-pane-overlaps-p cy ch y h)))
        (:up (and (= (+ cy ch) y) (zellij-pane-overlaps-p cx cw x w)))
        (:down (and (= (+ y h) cy) (zellij-pane-overlaps-p cx cw x w)))))))

(defun zellij-pane-activation (pane)
  (getf pane :activation-order))

(defun zellij-pane-focused (snapshot)
  (let ((focus (getf snapshot :focus)))
    (find focus (getf snapshot :panes) :key (lambda (pane) (getf pane :id)))))

(defun zellij-pane-directional-target (snapshot direction)
  (let* ((current (zellij-pane-focused snapshot))
         ;; Hidden siblings still own their tiled rectangles while a pane is
         ;; zoomed, and must remain focusable from the Pane keymap.
         (panes (remove-if-not #'zellij-pane-rect (getf snapshot :panes))))
    (when (and current (zellij-pane-rect current))
      (let ((best nil) (best-order nil) (best-id nil))
        (loop for pane in panes
              when (and (not (eql (getf pane :id) (getf current :id)))
                        (zellij-pane-adjacent-p pane current direction))
                do (let ((order (zellij-pane-activation pane))
                         (id (getf pane :id)))
                     (when (or (null best)
                               (> order best-order)
                               (and (= order best-order)
                                    (> id best-id)))
                       (setf best pane best-order order best-id id))))
        best))))

(defun zellij-pane-focus-action (snapshot direction)
  (let ((target (zellij-pane-directional-target snapshot direction)))
    (when target
      (list (ekko/extensions:action :focus :pane (getf target :id))))))

(defun zellij-pane-row-major (panes)
  (sort (copy-list panes)
        (lambda (a b)
          (destructuring-bind (ax ay width height) (zellij-pane-rect a)
            (declare (ignore width height))
            (destructuring-bind (bx by other-width other-height) (zellij-pane-rect b)
              (declare (ignore other-width other-height))
              (or (< ay by)
                  (and (= ay by)
                       (or (< ax bx)
                           (and (= ax bx)
                                (< (getf a :id) (getf b :id)))))))))))

(defun zellij-pane-switch-focus-action (snapshot)
  (let* ((panes (zellij-pane-row-major
                 (remove-if-not #'zellij-pane-rect (getf snapshot :panes))))
         (focus (getf snapshot :focus))
         (index (position focus panes :key (lambda (pane) (getf pane :id)))))
    (when (and panes index)
      (list (ekko/extensions:action
             :focus :pane (getf (nth (mod (1+ index) (length panes)) panes) :id))))))

(defun zellij-pane-move-action (snapshot target-id)
  "Swap the focused pane with TARGET-ID through the public layout action.

The pinned implementation swaps geometry overrides while fullscreen is active;
the public tree action path has not been paired for that case yet."
  (let ((focus (getf snapshot :focus)))
    ;; Pinned Tab move entry points reject tiled movement during fullscreen.
    (when (and (not (getf snapshot :zoom)) target-id (not (= focus target-id)))
      (list (ekko/extensions:action
             :set-layout :tree
             (ekko/layout:swap-panes (getf snapshot :layout) focus target-id))))))

(defun zellij-pane-move-cyclic-action (snapshot backwards)
  "Move the focused pane to the previous/next row-major pane."
  (let* ((panes (zellij-pane-row-major
                 (remove-if-not #'zellij-pane-rect (getf snapshot :panes))))
         (focus (getf snapshot :focus))
         (index (position focus panes :key (lambda (pane) (getf pane :id)))))
    (when (and panes index)
      (zellij-pane-move-action
       snapshot
       (getf (nth (mod (+ index (if backwards -1 1)) (length panes)) panes)
             :id)))))

(defun zellij-pane-move-direction-action (snapshot direction)
  "Move the focused pane into the direction target selected by tiled geometry."
  (let ((target (zellij-pane-directional-target snapshot direction)))
    (zellij-pane-move-action snapshot (and target (getf target :id)))))

(defun zellij-pane-failed-split-actions (snapshot pane-id)
  (append (when (getf snapshot :zoom)
            (list (ekko/extensions:action :zoom)))
          (list (ekko/extensions:action :set-keymap :name :normal)
                (ekko/extensions:action
                 :pane-note :pane pane-id :text "CAN'T SPLIT!"
                 :sgr '(0 1 38 5 124 49) :duration 1000))))

(defun zellij-pane-split-actions (axis &optional pane-id snapshot)
  (let ((room (and snapshot pane-id
                   (zellij-pane-room-p
                    (find pane-id (getf snapshot :panes)
                          :key (lambda (pane) (getf pane :id)))
                    axis))))
    (if (and snapshot pane-id (not room))
        (zellij-pane-failed-split-actions snapshot pane-id)
        (list (if pane-id
                  (ekko/extensions:action :split :pane pane-id :axis axis)
                  (ekko/extensions:action :split :axis axis))
              (ekko/extensions:action :set-keymap :name :normal)))))

(defun zellij-pane-room-p (pane axis)
  "Whether AXIS meets the pinned Pane split minimum.

  The daemon still performs the authoritative split validation.  The public
  snapshot lets the profile avoid a known failed request: a column split needs
  a base-layout width of at least ten cells and a row split needs a base-layout
  height of at least ten cells."
  (let ((rect (zellij-pane-rect pane)))
    (when rect
      (if (eq axis :columns)
          (>= (third rect) 10)
          (>= (fourth rect) 10)))))

(defun zellij-pane-find-room-for-new-pane (snapshot)
  "Return the pinned Pane NoPreference plan, or NIL when no pane qualifies.

Zellij first chooses the eligible pane with the greatest
ROWS * cell-height/cell-width * COLUMNS score.  Panes are considered in
ascending ID order and ties retain the lower ID.  It then chooses rows only
when the height-weighted width is larger and the height is over ten; columns
are the fallback when width is over ten.  There is no alternate-pane search."
  (let* ((viewport (getf snapshot :viewport))
         (cell-width (getf viewport :reported-cell-width))
         (cell-height (getf viewport :reported-cell-height))
         ;; Rust's f64::round rounds positive halves away from zero.  FLOOR
         ;; of ratio+1/2 has that behavior without CL's ties-to-even ROUND.
         (ratio (if (and (integerp cell-width) (> cell-width 0)
                         (integerp cell-height) (> cell-height 0))
                    (floor (+ (/ cell-height cell-width) 1/2))
                    4))
         (panes (sort (copy-list
                       (remove-if-not #'zellij-pane-rect (getf snapshot :panes))) #'<
                      :key (lambda (pane) (getf pane :id))))
         (winner nil)
         ;; The upstream search starts at zero, so a zero ratio produces no
         ;; candidate even when the terminal has otherwise eligible panes.
         (best-score 0))
    (dolist (pane panes)
      (let ((rect (zellij-pane-rect pane)))
        (when rect
          (let* ((width (third rect))
                 (height (fourth rect))
                 (score (* height ratio width)))
            (when (and (integerp width) (integerp height)
                       (>= width 5) (>= height 5)
                       (or (> width 10) (> height 10))
                       (> score best-score))
              (setf winner pane
                    best-score score))))))
    (when winner
      (let* ((rect (zellij-pane-rect winner))
             (width (third rect))
             (height (fourth rect)))
        (cond ((and (> (* height ratio) width) (> height 10))
               (list (getf winner :id) :rows))
              ((> width 10)
               (list (getf winner :id) :columns)))))))

(defun zellij-pane-no-preference-actions (snapshot)
  (let ((plan (zellij-pane-find-room-for-new-pane snapshot)))
    (if plan
        (zellij-pane-split-actions (second plan) (first plan) snapshot)
        ;; NoPreference silently declines when auto-layout has no room.  The
        ;; upstream insertion path still clears fullscreen before discovering
        ;; that no pane can be added, so preserve only that observable effect.
        (append (when (getf snapshot :zoom)
                  (list (ekko/extensions:action :zoom)))
                (list (ekko/extensions:action :set-keymap :name :normal))))))

(defun zellij-pane-close-actions (snapshot)
  ;; Zellij focuses the most recently active pane that remains after closing
  ;; the current one.  The daemon validates this replacement ID and applies
  ;; the close and focus change as one ordinary action.
  (let* ((focus (getf snapshot :focus))
         (survivors (remove focus (getf snapshot :panes)
                            :key (lambda (pane) (getf pane :id))))
         (replacement
           (loop with best = nil
                 for pane in survivors
                 for order = (getf pane :activation-order)
                 when (or (null best)
                          (> order (getf best :activation-order))
                          (and (= order (getf best :activation-order))
                               (> (getf pane :id) (getf best :id))))
                   do (setf best pane)
                 finally (return best))))
    (list (if replacement
              (ekko/extensions:action :close :focus (getf replacement :id))
              (ekko/extensions:action :close))
          (ekko/extensions:action :set-keymap :name :normal))))

;; RenamePane keeps the previous name separately because entering the mode
;; does not clear the editable label.  The daemon exposes this as ordinary
;; component state, so the policy remains reload-safe and independent of pane
;; process identity.
(defun zellij-pane-state-value (snapshot)
  (cdr (assoc "zellij-modes" (getf snapshot :component-state) :test #'equal)))

(defun zellij-pane-state-for-panes (state panes)
  (let ((ids (mapcar (lambda (pane) (getf pane :id)) panes)))
    (loop for entry in state
          when (and (listp entry) (= (length entry) 2)
                    (member (first entry) ids :test #'eql))
            collect (list (first entry) (second entry)))))

(defun zellij-pane-rename-state (snapshot pane-id old-name)
  (let* ((state (zellij-pane-state-for-panes
                 (or (zellij-pane-state-value snapshot) nil)
                 (getf snapshot :panes)))
         (without (remove pane-id state :key #'first :test #'eql)))
    (cons (list pane-id (or old-name "")) without)))

(defun zellij-pane-rename-enter-actions (snapshot)
  (let* ((pane (zellij-pane-focused snapshot))
         (id (and pane (getf pane :id)))
         (old (and pane (or (getf pane :name) ""))))
    (when pane
      (list (ekko/extensions:action
             :set-state :value (zellij-pane-rename-state snapshot id old))
            (ekko/extensions:action :set-keymap :name :rename)))))

(defun zellij-pane-rename-pop (text)
  (if (plusp (length text))
      (subseq text 0 (1- (length text)))
      text))

(defun zellij-pane-rename-decoded (bytes)
  (handler-case
      (let* ((octets (coerce bytes '(vector (unsigned-byte 8))))
             (text (sb-ext:octets-to-string octets :external-format :utf-8))
             ;; SBCL versions differ in whether malformed input signals or
             ;; inserts U+FFFD.  Round-tripping makes rejection deterministic.
             (encoded (sb-ext:string-to-octets text :external-format :utf-8)))
        (when (equal bytes (coerce encoded 'list)) text))
    (error () nil)))

(defun zellij-pane-rename-filter (text)
  (coerce (loop for char across text
                for code = (char-code char)
                unless (or (<= code #x1f)
                           (<= #x7f code #x9f)
                           (= code #x2028) (= code #x2029))
                  collect char)
          'string))

(defun zellij-pane-rename-input-action (snapshot event)
  (let* ((pane (zellij-pane-focused snapshot))
         (id (and pane (getf pane :id)))
         (old (and pane (or (getf pane :name) "")))
         (bytes (getf event :bytes)))
    (when (and pane (listp bytes) (every (lambda (byte) (typep byte '(integer 0 255))) bytes))
      (let ((new (cond ((or (equal bytes '(8)) (equal bytes '(127)))
                       (zellij-pane-rename-pop old))
                      (t (let ((decoded (zellij-pane-rename-decoded bytes)))
                           (and decoded (concatenate 'string old
                                                     (zellij-pane-rename-filter decoded))))))))
        (when new
          (list (ekko/extensions:action :rename :pane id :text new)))))))

(defun zellij-pane-rename-previous-action (snapshot)
  (let* ((pane (zellij-pane-focused snapshot))
         (id (and pane (getf pane :id)))
         (state (zellij-pane-state-for-panes
                 (or (zellij-pane-state-value snapshot) nil)
                 (getf snapshot :panes)))
         (entry (and id (find id state :key #'first :test #'eql))))
    (when pane
      (list (ekko/extensions:action :rename :pane id
                                    :text (or (and entry (second entry)) ""))
            (ekko/extensions:action :set-keymap :name :pane)))))
