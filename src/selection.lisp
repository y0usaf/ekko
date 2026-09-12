(in-package #:ekko/runtime)

(defun leave-copy (view pane)
  (let ((state (pane-state view (pane-id pane))))
    (setf (pane-view-copy-cells state) nil (pane-view-copy-lines state) nil
          (pane-view-search-input state) nil (pane-view-copy-flash-until state) nil
          (pane-view-copy-pointer state) nil (pane-view-copy-anchor state) nil
          (pane-view-copy-end state) nil)))

(defun expire-copy-flashes (view &optional (current (now)))
  (dolist (pane (view-panes view))
    (let ((state (pane-state view (pane-id pane))))
      (when (and (pane-view-copy-flash-until state) (>= current (pane-view-copy-flash-until state)))
        (setf (pane-view-copy-flash-until state) nil)
        (unless (pane-view-copy-pointer state) (leave-copy view pane))
        (incf (view-revision view))))))

(defun publish-copy (view text)
  (let ((bytes (text-bytes text)) (writer (view-wire view)))
    (when (> (length bytes) (* 1024 1024)) (error "Copy exceeds 1 MiB"))
    ;; Only this viewer writes terminal controls. The shared buffer survives it.
    (when writer (send-packet writer 23 bytes))
    (setf (session-clipboard (view-session view)) text
          (view-notice view) (if writer
                                 "Copied to Ekko buffer; terminal clipboard requested"
                                 "Copied to Ekko buffer"))))

(defun point-copy (view pane x y start)
  (let ((state (pane-state view (pane-id pane))))
    (setf (pane-view-copy-flash-until state) nil)
    (unless (pane-view-copy-lines state) (enter-copy view pane))
    (let ((point (cons (min (1- (length (pane-view-copy-lines state)))
                           (+ (pane-view-copy-top state) y)) x)))
      (setf (pane-view-copy-pointer state) t (pane-view-copy-end state) point
            (pane-view-copy-cursor state) (car point))
      (when (or start (null (pane-view-copy-anchor state)))
        (setf (pane-view-copy-anchor state) point)))))

(defun scroll-copy (view pane delta)
  (let ((state (pane-state view (pane-id pane))))
    (setf (pane-view-copy-flash-until state) nil)
    (unless (pane-view-copy-lines state)
      (enter-copy view pane)
      (setf (pane-view-copy-pointer state) t))
    (let ((bottom (max 0 (- (length (pane-view-copy-lines state)) (pane-view-rows state)))))
      (setf (pane-view-copy-top state) (max 0 (min bottom (+ (pane-view-copy-top state) delta)))
            (pane-view-copy-cursor state) (pane-view-copy-top state))
      (when (and (plusp delta) (= (pane-view-copy-top state) bottom) (pane-view-copy-pointer state))
        (leave-copy view pane)))))

(defun copy-point-before-p (a b)
  (or (< (car a) (car b)) (and (= (car a) (car b)) (<= (cdr a) (cdr b)))))

(defun copy-row-range (view pane row text)
  "Return string indices, including whole wide glyphs and combining marks."
  (let* ((state (pane-state view (pane-id pane)))
         (a (pane-view-copy-anchor state)) (b (pane-view-copy-end state))
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

(defun selected-copy-text (view pane)
  (let ((state (pane-state view (pane-id pane))))
    (if (pane-view-copy-anchor state)
        (let* ((a (pane-view-copy-anchor state)) (b (pane-view-copy-end state))
               (start (min (car a) (car b))) (end (max (car a) (car b))))
          (format nil "~{~A~^~%~}"
                  (loop for row from start to end for text = (aref (pane-view-copy-lines state) row)
                        collect (multiple-value-bind (left right) (copy-row-range view pane row text)
                                  (subseq text left right)))))
        (let ((start (min (or (pane-view-copy-mark state) (pane-view-copy-cursor state))
                          (pane-view-copy-cursor state)))
              (end (max (or (pane-view-copy-mark state) (pane-view-copy-cursor state))
                        (pane-view-copy-cursor state))))
          (format nil "~{~A~^~%~}" (coerce (subseq (pane-view-copy-lines state) start (1+ end)) 'list))))))

(defun copy-display-lines (view pane)
  (let ((state (pane-state view (pane-id pane))))
    (unless (pane-view-copy-pointer state) (move-copy view pane (pane-view-copy-cursor state)))
    (loop for row from (pane-view-copy-top state)
          below (min (length (pane-view-copy-lines state))
                     (+ (pane-view-copy-top state) (pane-view-rows state)))
          for text = (clip-copy-text (aref (pane-view-copy-lines state) row) (pane-view-cols state))
          collect
          (multiple-value-bind (left right)
              (if (pane-view-copy-pointer state)
                  (copy-row-range view pane row text)
                  (when (<= (min (or (pane-view-copy-mark state) (pane-view-copy-cursor state))
                                 (pane-view-copy-cursor state)) row
                            (max (or (pane-view-copy-mark state) (pane-view-copy-cursor state))
                                 (pane-view-copy-cursor state)))
                    (values 0 (length text))))
            (if (pane-view-copy-cells state)
                (let ((cells (aref (pane-view-copy-cells state) row)))
                  (cell-runs cells left right 0 (min (length cells) (pane-view-cols state))
                             (if (pane-view-copy-flash-until state) '(27 30 48 5 229) '(27 48 5 238))))
                ;; Help pages are plain text, unlike captured terminal rows.
                (list (list 0 text (if (and left right (< left right))
                                       (if (pane-view-copy-flash-until state) '(0 30 48 5 229) '(0 48 5 238))
                                       '(0)))))))))

(defun pointer-copy-input (view pane button x y up)
  "Translate terminal pointer events to the same validated public copy actions."
  (let* ((state (pane-state view (pane-id pane)))
         (modes (terminal-modes (pane-vt pane)))
         (tracking (some (lambda (mode) (gethash mode modes)) '(1000 1002 1003)))
         (wheel (logtest button 64)) (motion (logtest button 32))
         (left (zerop (logand button 3)))
         (cx (max 0 (min (1- (pane-view-cols state))
                         (- (floor (1- x) (view-cw view)) (pane-view-x state)))))
         (cy (max 0 (min (1- (pane-view-rows state))
                         (- (floor (1- y) (view-ch view)) (pane-view-y state))))))
    (when (and tracking (not (pane-view-copy-lines state))) (return-from pointer-copy-input nil))
    (cond
      ((and wheel (not up))
       (apply-actions view :pointer (list (list :copy-scroll :pane (pane-id pane)
                                               :delta (if (oddp button) 3 -3))) nil nil))
      ((and left (or (not motion) (eq pane (view-drag view))))
       (when (or (not up) (pane-view-copy-anchor state))
         (apply-actions view :pointer (list (list :copy-point :pane (pane-id pane)
                                                :x cx :y cy :start (and (not up) (not motion)))) nil nil)
         (when up (apply-actions view :pointer (list (list :copy-selection :pane (pane-id pane))) nil nil))))
      (t nil))
    t))
