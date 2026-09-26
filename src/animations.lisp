(in-package #:ekko/runtime)

;; Motion is presentation. Logical layout commits once; every effect here is a
;; function of committed state and the clock, and ticks never touch
;; application VT or PTY geometry. Progress is continuous: while anything
;; moves the daemon publishes at display rate, and the client's row diff sends
;; only the rows that changed. Cells cannot move by less than a cell, but
;; truecolour can change by any amount, so every effect that can be a fade is.
(defparameter +frame-seconds+ 1/60)
(defun ease-out (p) (- 1 (expt (- 1 (max 0 (min 1 p))) 3)))
(defun animation-seconds (view &optional (scale 1))
  (* scale (/ (option (view-daemon view) :window-animation-ms 0) 1000)))
(defun animation-progress (started seconds)
  (if (plusp seconds) (min 1 (/ (- (now) started) seconds)) 1))

;; Colour arithmetic on SGR parameter lists.
(defparameter +system-colours+
  '((0 0 0) (205 0 0) (0 205 0) (205 205 0) (0 0 238) (205 0 205) (0 205 205) (229 229 229)
    (127 127 127) (255 0 0) (0 255 0) (255 255 0) (92 92 255) (255 0 255) (0 255 255) (255 255 255)))
(defun palette-rgb (index)
  "An xterm 256-colour INDEX as (r g b); the 16 system colours take xterm's defaults."
  (cond ((< index 16) (nth index +system-colours+))
        ((< index 232) (let ((i (- index 16)))
                         (mapcar (lambda (c) (if (zerop c) 0 (+ 55 (* 40 c))))
                                 (list (floor i 36) (mod (floor i 6) 6) (mod i 6)))))
        (t (let ((v (+ 8 (* 10 (- index 232))))) (list v v v)))))
(defun rgb (colour) (if (integerp colour) (palette-rgb colour) colour))
(defun split-sgr (sgr)
  "SGR as (values fg bg rest): colours as (r g b) or nil for the default, REST
the remaining codes in order."
  (let (fg bg rest)
    (loop while sgr for code = (pop sgr) do
      (case code
        ((38 48 58)
         (let* ((kind (pop sgr))
                (colour (case kind
                          (5 (let ((index (pop sgr))) (and index (palette-rgb index))))
                          (2 (let ((c (list (pop sgr) (pop sgr) (pop sgr)))) (and (every #'integerp c) c))))))
           (case code (38 (setf fg colour)) (48 (setf bg colour)))))
        (39 (setf fg nil)) (49 (setf bg nil))
        (t (cond ((<= 30 code 37) (setf fg (palette-rgb (- code 30))))
                 ((<= 90 code 97) (setf fg (palette-rgb (- code 82))))
                 ((<= 40 code 47) (setf bg (palette-rgb (- code 40))))
                 ((<= 100 code 107) (setf bg (palette-rgb (- code 92))))
                 (t (push code rest))))))
    (values fg bg (nreverse rest))))
(defun join-sgr (fg bg rest)
  (append rest (when fg (list* 38 2 fg)) (when bg (list* 48 2 bg))))
(defun mix (a b p)
  (mapcar (lambda (x y) (round (+ x (* (- y x) p)))) a b))
(defun blend-sgr (from to p)
  "Style TO with its colours P of the way from FROM's. A colour that one side
leaves to the terminal default cannot be interpolated; it switches halfway."
  (if (or (>= p 1) (equal from to))
      to
      (multiple-value-bind (f1 b1 r1) (split-sgr from)
        (multiple-value-bind (f2 b2 r2) (split-sgr to)
          (flet ((pick (a b) (cond ((and a b) (mix a b p)) ((< p 1/2) a) (t b))))
            (join-sgr (pick f1 f2) (pick b1 b2) (if (< p 1/2) r1 r2)))))))
(defun faded-sgr (sgr alpha ground)
  "SGR seen at opacity ALPHA over GROUND."
  (let ((ground (rgb ground)))
    (multiple-value-bind (fg bg rest) (split-sgr sgr)
      (join-sgr (and fg (mix ground fg alpha)) (mix ground (or bg ground) alpha) rest))))

;; Restyle crossfades. A decoration span that keeps its place and text but
;; changes style (focus moving between title bars, a taskbar entry becoming
;; active) blends from the style it showed to the new one. Keyed by owner,
;; position and text, so anything that moves or rewrites itself just switches.
(defun span-key (owner span)
  (list owner (getf span :x) (getf span :y) (getf span :rows 1) (getf span :text)))
(defun span-style (span) (list (getf span :sgr) (getf span :gradient)))
(defun shown-style (view key style)
  (let ((tween (and (view-tweens view) (gethash key (view-tweens view)))))
    (if (and tween (equal (second tween) style))
        (destructuring-bind (from to started) tween
          (let ((p (ease-out (animation-progress started (animation-seconds view)))))
            (list (blend-sgr (first from) (first to) p)
                  (and (second to) (blend-sgr (or (second from) (first from)) (second to) p)))))
        style)))
(defun note-decorations (view owner old new)
  "Start crossfades, exit ghosts and slide bookkeeping as OWNER's spans change."
  (let ((seconds (animation-seconds view)))
    (push (cons owner (now)) (view-redecorated view))
    (setf (view-redecorated view) (remove-duplicates (view-redecorated view) :key #'car :test #'equal :from-end t))
    (when (and (plusp seconds) old)
      (let ((before (make-hash-table :test 'equal)))
        (dolist (span old)
          (let ((key (span-key owner span)))
            (setf (gethash key before) (shown-style view key (span-style span)))))
        (dolist (span new)
          (let* ((key (span-key owner span)) (from (gethash key before)) (to (span-style span)))
            (when (and from (not (equal from to)))
              (unless (view-tweens view) (setf (view-tweens view) (make-hash-table :test 'equal)))
              (setf (gethash key (view-tweens view)) (list from to (now)))))))
      ;; Entrance groups that disappear leave through the way they came.
      (let ((kept (loop for span in new when (getf span :enter) collect (first (getf span :enter)))))
        (loop for span in old for enter = (getf span :enter)
              when (and enter (not (member (first enter) kept :test #'equal)))
                do (let ((ghost (copy-list span)))
                     (remf ghost :pane)
                     (push (list owner ghost (now)) (view-ghosts view))))))))
(defun ghost-spans (view)
  "Departed entrance spans, as (owner span dx dy alpha)."
  (loop with seconds = (animation-seconds view 3/4)
        for (owner span started) in (view-ghosts view)
        for p = (ease-out (animation-progress started seconds))
        when (< p 1)
          collect (destructuring-bind (id dx dy) (getf span :enter)
                    (declare (ignore id))
                    (list owner span (round (* dx p 1/2)) (round (* dy p 1/2)) (- 1 p)))))

;; Glows: pointer and keyboard highlight that arrives quickly and leaves
;; slowly, so a sweep across a taskbar or a menu trails light behind it.
;; Each glow is (key level started target span owner).
(defun glow-level (view glow)
  (destructuring-bind (key level started target &rest more) glow
    (declare (ignore key more))
    (let ((seconds (animation-seconds view (if target 1/3 3/2))))
      (if (plusp seconds)
          (let ((step (/ (- (now) started) seconds)))
            (if target (min 1 (+ level step)) (max 0 (- level step))))
          (if target 1 0)))))
(defun set-glow (view key target &optional span owner)
  (let ((old (find key (view-glows view) :key #'first :test #'equal)))
    (unless (and old (eq (fourth old) target))
      (setf (view-glows view)
            (cons (list key (if old (glow-level view old) 0) (now) target span owner)
                  (remove old (view-glows view))))
      (incf (view-revision view)))))
(defun glow-at (view key)
  (let ((glow (find key (view-glows view) :key #'first :test #'equal)))
    (if glow (glow-level view glow) 0)))

;; Entrance motion: spans that declare (:enter (id dx dy)) start offset by
;; (dx dy) and transparent, and ease into place over :window-animation-ms.
;; Each motion is (id started). A group re-enters after it disappears.
(defun motion-offset (view span)
  "SPAN's current (values dx dy alpha), starting its group's motion on first sight."
  (let ((enter (getf span :enter)))
    (if (null enter)
        (values 0 0 nil)
        (destructuring-bind (id dx dy) enter
          (let* ((motion (or (assoc id (view-motions view) :test #'equal)
                             (first (push (list id (now)) (view-motions view)))))
                 (p (ease-out (animation-progress (second motion) (animation-seconds view)))))
            (values (round (* dx (- 1 p))) (round (* dy (- 1 p))) (and (< p 1) p)))))))

;; Window motion. A window shows at its target: the rectangle a drag holds it
;; at, else its committed place. A motion carries it from where it showed to
;; its target, sliding, and growing or shrinking by clipping content that is
;; already at its new size. Chrome follows by nine-slice: rows that span the
;; window stretch in their longest run of one character, spans nearer an
;; edge keep their distance to it, borders that span its height grow with it.
;; A motion is (id from started seconds ref . spans): the rectangle it starts
;; from, and the window's chrome as it was, drawn for rectangle REF, which
;; carries the window until its owners redraw and covers any part of the
;; frame that was off screen on one side of the move.
(defun lerp-rect (a b p) (mapcar (lambda (x y) (round (+ x (* (- y x) p)))) a b))
(defun committed-rect (view id)
  (let ((state (gethash id (view-pane-states view))))
    (and state (list (pane-view-outer-x state) (pane-view-outer-y state)
                     (pane-view-outer-cols state) (pane-view-outer-rows state)))))
(defun drag-rects (view)
  "The rectangles an active drag holds windows at, as (id . rect)."
  (let ((drag (view-window-drag view)))
    (when (and drag (window-drag-moved drag))
      (if (window-drag-split drag)
          (window-drag-preview drag)
          (list (cons (pane-id (window-drag-pane drag)) (window-drag-free drag)))))))
(defun target-rect (view id)
  (or (rest (assoc id (drag-rects view))) (committed-rect view id)))
(defun shown-rect (view id)
  (let ((target (target-rect view id)) (motion (assoc id (view-slides view))))
    (if (and motion target)
        (destructuring-bind (from started seconds &rest more) (rest motion)
          (declare (ignore more))
          (lerp-rect from target (ease-out (animation-progress started seconds))))
        target)))
(defun window-moving-p (view) (or (view-slides view) (view-window-drag view)))
(defun pane-shift (view id)
  "Pane ID shown against its committed place, as (values dx dy dcols drows)."
  (let ((committed (and (window-moving-p view) (committed-rect view id))))
    (if committed
        (values-list (mapcar #'- (shown-rect view id) committed))
        (values 0 0 0 0))))
(defun outer-places (view &optional placed)
  "Shown outer rectangles as (id x y cols rows): of the visible windows, or
with PLACED of every window the layout places."
  (loop for pane in (if placed
                        (loop for placement in (view-layout-placements view)
                              for pane = (pane-by-id (view-daemon view) (getf placement :pane))
                              when (and pane (getf placement :visible t)) collect pane)
                        (visible-panes view))
        collect (cons (pane-id pane) (shown-rect view (pane-id pane)))))
(defun pane-chrome (view id)
  "Every owner's spans that belong to window ID, as (owner . span)."
  (loop for (owner . spans) in (view-decorations view)
        append (loop for span in spans
                     when (eql (or (getf span :pane) (getf span :follows)) id) collect (cons owner span))))
(defun move-window (view id from seconds &optional ref spans)
  (setf (view-slides view)
        (cons (list* id from (now) seconds ref spans) (remove id (view-slides view) :key #'first))))
(defun grown-from (rect before)
  "Where a new window at RECT grows from: the far edge of the window it split."
  (destructuring-bind (x y w h) rect
    (loop for (nil bx by bw bh) in before
          when (and (< bx (+ x w)) (< x (+ bx bw)) (< by (+ y h)) (< y (+ by bh)))
            return (cond ((> x bx) (list (+ bx bw) y 0 h))
                         ((> y by) (list x (+ by bh) w 0))
                         ((< (+ x w) (+ bx bw)) (list x y 0 h))
                         (t (list x y w 0))))))
(defun start-window-motions (view before committed)
  "Move each window from where BEFORE showed it; COMMITTED holds the places
the last layout gave, and a window whose place is unchanged keeps its motion."
  (setf (view-slides view)
        (when (and (plusp (animation-seconds view)) (view-wire view))
          (loop for placement in (view-layout-placements view)
                for id = (getf placement :pane)
                for new = (committed-rect view id)
                for shown = (rest (assoc id before))
                for old = (rest (assoc id committed))
                for motion = (assoc id (view-slides view))
                when (and new (getf placement :visible t))
                  append (cond ((and shown (equal old new)) (and motion (list motion)))
                               (shown (unless (equal shown new)
                                        (list (list* id shown (now) (animation-seconds view 3/2)
                                                     old (pane-chrome view id)))))
                               (t (let ((from (grown-from new before)))
                                    (and from (list (list* id from (now) (animation-seconds view 3/2) nil))))))))))
(defun stretch-text (text delta)
  "TEXT DELTA cells wider or narrower, changed inside its longest run of one character."
  (if (or (zerop delta) (zerop (length text)))
      text
      (let ((best 0) (best-length 0) (start 0))
        (loop for i from 1 to (length text)
              when (or (= i (length text)) (char/= (char text i) (char text start)))
                do (when (> (- i start) best-length) (setf best start best-length (- i start)))
                   (setf start i))
        (if (plusp delta)
            (concatenate 'string (subseq text 0 best) (make-string delta :initial-element (char text best))
                         (subseq text best))
            (let* ((cut (min best-length (- delta)))
                   (shrunk (concatenate 'string (subseq text 0 best) (subseq text (+ best cut)))))
              (subseq shrunk 0 (max 0 (- (length shrunk) (- (- delta) cut)))))))))
(defun fit-span (span ref rect)
  "SPAN, drawn for window rectangle REF, fitted by nine-slice to RECT; nil when nothing is left."
  (if (or (null ref) (equal ref rect))
      span
      (destructuring-bind (rx ry rw rh) ref
        (destructuring-bind (tx ty tw th) rect
          (let* ((x (getf span :x)) (y (getf span :y)) (rows (getf span :rows 1))
                 (width (span-width span)) (text (getf span :text))
                 (left (- x rx)) (right (- (+ rx rw) (+ x width)))
                 (top (- y ry)) (bottom (- (+ ry rh) (+ y rows)))
                 (wide (and (> width 2) (>= width (- rw 2))))
                 (tall (and (> rows 1) (>= rows (- rh 2))))
                 (text (if wide (stretch-text text (- tw rw)) text))
                 (rows (if tall (+ rows (- th rh)) rows)))
            (when (and (plusp (length text)) (plusp rows))
              (let ((fitted (copy-list span)))
                (setf (getf fitted :x) (if (or wide (<= left right)) (+ tx left) (- (+ tx tw) right width))
                      (getf fitted :y) (if (or tall (<= top bottom)) (+ ty top) (- (+ ty th) bottom rows))
                      (getf fitted :text) text)
                (when (or tall (getf span :rows)) (setf (getf fitted :rows) rows))
                fitted)))))))
(defun window-span (view owner span)
  "OWNER's live SPAN where its window shows now; nil when it has no room."
  (let ((id (and (window-moving-p view) (or (getf span :pane) (getf span :follows)))))
    (if id
        (let* ((motion (assoc id (view-slides view)))
               (ref (if (and motion (fifth motion) (not (redecorated-since-p view owner (third motion))))
                        (fifth motion)
                        (committed-rect view id))))
          (fit-span span ref (shown-rect view id)))
        span)))

;; Short window transitions use the overlay channel: a minimize or restore
;; flies the window's title bar between the window and its taskbar entry,
;; fading as it lands, and other geometry changes sweep an outline.
(defstruct transition style from to sgr started duration)
(defun pane-outer-rect (view pane)
  (let ((state (pane-state view (pane-id pane))))
    (list (pane-view-outer-x state) (pane-view-outer-y state)
          (pane-view-outer-cols state) (pane-view-outer-rows state))))
(defun capture-transition (view action)
  (let* ((pane (if (getf (rest action) :pane)
                   (pane-by-id (view-daemon view) (getf (rest action) :pane))
                   (focused-pane view)))
         (state (when pane (pane-state view (pane-id pane))))
         (op (first action)))
    (when (and pane (plusp (animation-seconds view)) (view-wire view)
               (or (member op '(:minimize :restore :zoom :float :tile :place-window))
                   (and (eq op :focus) (pane-view-minimized state))))
      (let* ((spans (loop for entry in (view-decorations view) append (rest entry)))
             (dock (find-if (lambda (span)
                              (and (= (getf span :y) (1- (view-rows view)))
                                   (eql (getf (rest (getf span :action)) :pane) (pane-id pane)))) spans))
             (header (find-if (lambda (span)
                                (and (= (getf span :x) (pane-view-outer-x state))
                                     (= (getf span :y) (pane-view-outer-y state))
                                     (eql (getf (rest (getf span :action)) :pane) (pane-id pane)))) spans))
             (dock-rect (if dock (list (getf dock :x) (getf dock :y) (max 1 (span-width dock)) 1)
                            (list (floor (view-cols view) 2) (1- (view-rows view)) 1 1)))
             (sgr (copy-list (getf (or header dock) :sgr '(0 38 5 245))))
             (caption (and (member op '(:minimize :restore :focus)) (nth-value 1 (split-sgr sgr)))))
        (list pane op (if caption :caption :outline)
              (if (pane-view-minimized state) dock-rect (pane-outer-rect view pane)) dock-rect
              (if caption (join-sgr nil caption nil) sgr))))))
(defun start-transition (view captured)
  (when captured
    (destructuring-bind (pane op style from dock sgr) captured
      (let ((to (if (eq op :minimize) dock (pane-outer-rect view pane))))
        (unless (equal from to)
          (setf (view-transition view)
                (make-transition :style style :from from :to to :sgr sgr :started (now)
                                 :duration (animation-seconds view (if (eq style :caption) 3/2 1)))))))))
(defun transition-overlays (view)
  (let ((transition (view-transition view)))
    (when transition
      (let* ((p (ease-out (animation-progress (transition-started transition)
                                              (transition-duration transition))))
             (rect (mapcar (lambda (a b) (round (+ a (* (- b a) p))))
                           (transition-from transition) (transition-to transition)))
             (sgr (transition-sgr transition)))
        (graphics-safe-outlines view
          (if (eq (transition-style transition) :caption)
              (list (list (first rect) (second rect) (make-string (max 1 (third rect)) :initial-element #\Space)
                          sgr (- 1 (* p p))))
              (outline-spans rect sgr)))))))

;; One tick advances everything above. Each tick republishes while anything
;; moves, and once more after it stops so the last frame is the final state.
(defun animating-p (view)
  (or (view-transition view) (view-slides view) (view-ghosts view) (view-popup-ghost view)
      (some (lambda (glow) (let ((level (glow-level view glow))) (if (fourth glow) (< level 1) (> level 0))))
            (view-glows view))
      (and (view-tweens view) (plusp (hash-table-count (view-tweens view))))
      (some (lambda (motion) (< (animation-progress (second motion) (animation-seconds view)) 1))
            (view-motions view))))
(defun finished-p (started seconds) (>= (animation-progress started seconds) 1))
(defun tick-animations (view)
  ;; A group that disappears re-enters when it next appears.
  (let ((present (loop for span in (append (loop for entry in (view-decorations view) append (rest entry))
                                            (and (view-popup view) (popup-spans (view-popup view))))
                       when (getf span :enter) collect (first (getf span :enter)))))
    (setf (view-motions view)
          (remove-if-not (lambda (motion) (member (first motion) present :test #'equal)) (view-motions view))))
  (let ((moving (animating-p view)) (seconds (animation-seconds view)))
    (when moving
      (let ((transition (view-transition view)))
        (when (and transition (finished-p (transition-started transition) (transition-duration transition)))
          (setf (view-transition view) nil)))
      (when (and (view-popup-ghost view) (finished-p (second (view-popup-ghost view)) (animation-seconds view 3/4)))
        (setf (view-popup-ghost view) nil))
      (setf (view-slides view)
            (remove-if (lambda (motion) (finished-p (third motion) (fourth motion))) (view-slides view))
            (view-ghosts view)
            (remove-if (lambda (ghost) (finished-p (third ghost) (animation-seconds view 3/4))) (view-ghosts view))
            (view-glows view)
            (remove-if (lambda (glow) (and (not (fourth glow)) (zerop (glow-level view glow)))) (view-glows view)))
      (when (view-tweens view)
        (maphash (lambda (key tween) (when (finished-p (third tween) seconds) (remhash key (view-tweens view))))
                 (view-tweens view))))
    (when (or moving (view-animating view)) (incf (view-revision view)))
    (setf (view-animating view) moving)))
