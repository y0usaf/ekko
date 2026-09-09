(in-package #:ekko/runtime)

(defun leave-copy (pane)
  (setf (pane-copy-cells pane) nil (pane-copy-lines pane) nil (pane-search-input pane) nil
        (pane-copy-flash-until pane) nil
        (pane-copy-pointer pane) nil (pane-copy-anchor pane) nil (pane-copy-end pane) nil))

(defun expire-copy-flashes (session &optional (current (now)))
  (dolist (pane (session-panes session))
    (when (and (pane-copy-flash-until pane) (>= current (pane-copy-flash-until pane)))
      (setf (pane-copy-flash-until pane) nil)
      (unless (pane-copy-pointer pane) (leave-copy pane))
      (incf (session-revision session)))))

(defun publish-copy (session text)
  (let ((bytes (text-bytes text)) (writer (session-writer session)))
    (when (> (length bytes) (* 1024 1024)) (error "Copy exceeds 1 MiB"))
    ;; Only the viewer writes terminal controls. Legacy clients retain the buffer.
    (when (and writer (>= (wire-version writer) 12)) (send-packet writer 23 bytes))
    (setf (session-clipboard session) text
          (session-notice session) (if (and writer (>= (wire-version writer) 12))
                                       "Copied to Ekko buffer; terminal clipboard requested"
                                       "Copied to Ekko buffer"))))

(defun point-copy (pane x y start)
  (setf (pane-copy-flash-until pane) nil)
  (unless (pane-copy-lines pane) (enter-copy pane))
  (let ((point (cons (min (1- (length (pane-copy-lines pane))) (+ (pane-copy-top pane) y)) x)))
    (setf (pane-copy-pointer pane) t (pane-copy-end pane) point
          (pane-copy-cursor pane) (car point))
    (when (or start (null (pane-copy-anchor pane))) (setf (pane-copy-anchor pane) point))))

(defun scroll-copy (pane delta)
  (setf (pane-copy-flash-until pane) nil)
  (unless (pane-copy-lines pane)
    (enter-copy pane)
    (setf (pane-copy-pointer pane) t))
  (let ((bottom (max 0 (- (length (pane-copy-lines pane)) (terminal-rows (pane-vt pane))))))
    (setf (pane-copy-top pane) (max 0 (min bottom (+ (pane-copy-top pane) delta)))
          (pane-copy-cursor pane) (pane-copy-top pane))
    (when (and (plusp delta) (= (pane-copy-top pane) bottom) (pane-copy-pointer pane))
      (leave-copy pane))))

(defun copy-point-before-p (a b)
  (or (< (car a) (car b)) (and (= (car a) (car b)) (<= (cdr a) (cdr b)))))

(defun copy-row-range (pane row text)
  "Return string indices, including whole wide glyphs and combining marks."
  (let* ((a (pane-copy-anchor pane)) (b (pane-copy-end pane))
         (forward (and a b (copy-point-before-p a b)))
         (lo (if forward a b)) (hi (if forward b a)))
    (when (and lo hi (<= (car lo) row (car hi)))
      (let ((left (if (= row (car lo)) (cdr lo) 0))
            (right (if (= row (car hi)) (1+ (cdr hi)) most-positive-fixnum))
            (column 0) (start nil) (end nil))
        (loop for c across text for i from 0 for width = (ekko/vt::character-width c) do
          (when (if (zerop width) (and end (= end i))
                    (and (< column right) (> (+ column width) left)))
            (unless start (setf start i))
            (setf end (1+ i)))
          (incf column width))
        (values (or start 0) (or end 0))))))

(defun selected-copy-text (pane)
  (if (pane-copy-anchor pane)
      (let* ((a (pane-copy-anchor pane)) (b (pane-copy-end pane))
             (start (min (car a) (car b))) (end (max (car a) (car b))))
        (format nil "~{~A~^~%~}"
                (loop for row from start to end for text = (aref (pane-copy-lines pane) row)
                      collect (multiple-value-bind (left right) (copy-row-range pane row text)
                                (subseq text left right)))))
      (let ((start (min (or (pane-copy-mark pane) (pane-copy-cursor pane)) (pane-copy-cursor pane)))
            (end (max (or (pane-copy-mark pane) (pane-copy-cursor pane)) (pane-copy-cursor pane))))
        (format nil "~{~A~^~%~}" (coerce (subseq (pane-copy-lines pane) start (1+ end)) 'list)))))

(defun copy-display-lines (pane)
  (unless (pane-copy-pointer pane) (move-copy pane (pane-copy-cursor pane)))
  (loop for row from (pane-copy-top pane)
        below (min (length (pane-copy-lines pane)) (+ (pane-copy-top pane) (terminal-rows (pane-vt pane))))
        for text = (clip-copy-text (aref (pane-copy-lines pane) row) (terminal-cols (pane-vt pane)))
        collect
        (multiple-value-bind (left right)
            (if (pane-copy-pointer pane)
                (copy-row-range pane row text)
                (when (<= (min (or (pane-copy-mark pane) (pane-copy-cursor pane)) (pane-copy-cursor pane)) row
                          (max (or (pane-copy-mark pane) (pane-copy-cursor pane)) (pane-copy-cursor pane)))
                  (values 0 (length text))))
          (if (pane-copy-cells pane)
              (let ((cells (aref (pane-copy-cells pane) row)))
                (cell-runs cells left right 0 (min (length cells) (terminal-cols (pane-vt pane)))
                           (if (pane-copy-flash-until pane) '(27 30 48 5 229) '(27 48 5 238))))
              ;; Help pages are plain text, unlike captured terminal rows.
              (list (list 0 text (if (and left right (< left right))
                                     (if (pane-copy-flash-until pane) '(0 30 48 5 229) '(0 48 5 238))
                                     '(0))))))))

(defun pointer-copy-input (session pane button x y up)
  "Translate terminal pointer events to the same validated public copy actions."
  (let* ((modes (terminal-modes (pane-vt pane)))
         (tracking (some (lambda (mode) (gethash mode modes)) '(1000 1002 1003)))
         (wheel (logtest button 64)) (motion (logtest button 32))
         (left (zerop (logand button 3)))
         (cx (max 0 (min (1- (terminal-cols (pane-vt pane)))
                         (- (floor (1- x) (session-cw session)) (pane-x pane)))))
         (cy (max 0 (min (1- (terminal-rows (pane-vt pane)))
                         (- (floor (1- y) (session-ch session)) (pane-y pane))))))
    (when (and tracking (not (pane-copy-lines pane))) (return-from pointer-copy-input nil))
    (cond
      ((and wheel (not up))
       (apply-actions session :pointer (list (list :copy-scroll :pane (pane-id pane)
                                                   :delta (if (oddp button) 3 -3))) nil nil))
      ((and left (or (not motion) (eq pane (session-drag session))))
       (when (or (not up) (pane-copy-anchor pane))
         (apply-actions session :pointer (list (list :copy-point :pane (pane-id pane)
                                                    :x cx :y cy :start (and (not up) (not motion)))) nil nil)
         (when up (apply-actions session :pointer '((:copy-selection)) nil nil))))
      (t nil))
    t))
