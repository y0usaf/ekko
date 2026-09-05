(in-package #:ekko/runtime)

(defparameter *terminal-enter*
  (format nil "~C[?1049h~C[?25l~C[?1003h~C[?1006h~C[?1016h~C[?1004h~C[?2004h~C[>1u~C[16t"
          #1=(code-char 27) #1# #1# #1# #1# #1# #1# #1# #1#))
(defparameter *terminal-leave*
  (format nil "~C[?2026l~C[<u~C[?1003l~C[?1006l~C[?1016l~C[?1004l~C[?2004l~C[0m~C[?25h~C[?1049l"
          #1=(code-char 27) #1# #1# #1# #1# #1# #1# #1# #1# #1#))
(defstruct viewer io connection scene (assets (make-hash-table :test 'equal))
  (pending-assets (make-hash-table :test 'equal))
  (transport :unknown) probe-deadline awaiting-scene
  (uploads (make-hash-table))
  (drawn (make-hash-table :test 'equal))
  ;; Complete per-row terminal sequences from the last frame.  Keeping these
  ;; as strings lets us skip all cursor movement and text output for rows that
  ;; did not change.
  (row-cache (make-array 0)) (row-cache-cols 0) (row-cache-rows 0)
  (ids (ekko/client:make-attachment :max-mappings 4096 :max-id 4294967294))
  (input-state :ground) (input (make-array 0 :element-type '(unsigned-byte 8) :adjustable t :fill-pointer 0))
  (input-at 0) (paste nil) (done nil) size (cw 8) (ch 16))
(defun terminal-write (viewer text) (queue-bytes (viewer-io viewer) (text-bytes text)))
(defun send-size (viewer &optional queried-cw queried-ch)
  (let* ((size (terminal-size 0)) (cols (first size)) (rows (second size))
         (cw (or queried-cw (and (plusp cols) (plusp (third size)) (floor (third size) cols)) (viewer-cw viewer)))
         (ch (or queried-ch (and (plusp rows) (plusp (fourth size)) (floor (fourth size) rows)) (viewer-ch viewer))))
    (setf cols (max 5 (min 500 cols)) rows (max 4 (min 300 rows)) cw (max 1 cw) ch (max 1 ch))
    (when (or (not (equal size (viewer-size viewer))) (/= cw (viewer-cw viewer)) (/= ch (viewer-ch viewer)))
      (setf (viewer-size viewer) size (viewer-cw viewer) cw (viewer-ch viewer) ch)
      (send-packet (viewer-connection viewer) 1 (integers (list +wire-version+ cols rows cw ch))))))
(defun clipped-image (pane placement asset cw ch)
  (destructuring-bind (id x y cols rows &rest rest) pane
    (declare (ignore id rest))
    (destructuring-bind (child generation ix iy scale-cols scale-rows) placement
      (declare (ignore child generation))
      (when (or (plusp scale-cols) (plusp scale-rows)) (return-from clipped-image nil))
      (destructuring-bind (generation w h format data) asset
        (declare (ignore generation format data))
        (let* ((destination (ekko/scene:make-rect (+ (* x cw) ix) (+ (* y ch) iy) w h))
               (bounds (ekko/scene:make-rect (* x cw) (* y ch) (* cols cw) (* rows ch)))
               (cut (ekko/scene:rect-intersection destination bounds)))
          (when cut
            (list (ekko/scene:rect-x cut) (ekko/scene:rect-y cut)
                  (- (ekko/scene:rect-x cut) (ekko/scene:rect-x destination))
                  (- (ekko/scene:rect-y cut) (ekko/scene:rect-y destination))
                  (ekko/scene:rect-width cut) (ekko/scene:rect-height cut))))))))
(defun delete-outer (viewer id)
  (terminal-write viewer (format nil "~C_Ga=d,d=I,i=~D,q=2~C\\" (code-char 27) id (code-char 27))))
(defun compressed-asset-data (data)
  (if (local-asset-p data)
      (with-open-file (in (local-asset-path data) :element-type '(unsigned-byte 8))
        (unless (= (file-length in) (local-asset-size data)) (error "Changed local asset"))
        (let ((bytes (octets (local-asset-size data))))
          (unless (= (read-sequence bytes in) (length bytes)) (error "Truncated local asset"))
          (compress-bytes bytes))) data))

(defun probe-local-transport (viewer)
  (when (eq (viewer-transport viewer) :probing) (return-from probe-local-transport t))
  (when (eq (viewer-transport viewer) :unknown)
    (let ((asset (loop for value being the hash-values of (viewer-assets viewer)
                      when (local-asset-p (fifth value)) return value)))
      (when asset
        (destructuring-bind (generation width height format data) asset
          (declare (ignore generation))
          (terminal-write viewer
            (format nil "~C_Ga=q,t=f,f=~D,s=~D,v=~D,i=4294967295;~A~C\\"
                    #\Esc format width height (base64-encode (text-bytes (local-asset-path data))) #\Esc)))
        (setf (viewer-transport viewer) :probing (viewer-probe-deadline viewer) (+ (now) 1))
        t))))

(defun host-graphics-reply (viewer bytes)
  ;; Replies terminate here, never in a child PTY. Only outstanding IDs can
  ;; advance a probe or release a presentation lease.
  (when (and (> (length bytes) 5) (= (aref bytes 2) 71))
    (let* ((semi (position 59 bytes))
           (keys (and semi (ekko/graphics::header (map 'string #'code-char (subseq bytes 3 semi)))))
           (id (and keys (ekko/graphics::number-key keys #\i)))
           (ok (and semi (equalp (subseq bytes (1+ semi) (- (length bytes) 2)) #(79 75)))))
      (cond
        ((and (eql id 4294967295) (eq (viewer-transport viewer) :probing))
         (setf (viewer-transport viewer) (if ok :file :inline) (viewer-probe-deadline viewer) nil))
        ((and id (gethash id (viewer-uploads viewer))
              (= (ekko/graphics::number-key keys #\p) 1))
         (unless ok (error "Host rejected local image ~D" id))
         (remhash id (viewer-uploads viewer)))))))

(defun upload-outer (viewer id asset crop cw ch)
  (destructuring-bind (generation width height format data) asset
    (declare (ignore generation))
    (when (and (local-asset-p data) (eq (viewer-transport viewer) :file))
      (destructuring-bind (x y sx sy w h) crop
        (terminal-write viewer
          (format nil "~C[~D;~DH~C_Ga=T,t=f,f=~D,s=~D,v=~D,i=~D,p=1,C=1,q=0,x=~D,y=~D,w=~D,h=~D,X=~D,Y=~D;~A~C\\"
                  #\Esc (1+ (floor y ch)) (1+ (floor x cw)) #\Esc format width height id
                  sx sy w h (mod x cw) (mod y ch)
                  (base64-encode (text-bytes (local-asset-path data))) #\Esc))
        (setf (gethash id (viewer-uploads viewer)) (now)))
      (return-from upload-outer))
    (destructuring-bind (x y sx sy w h) crop
      (let* ((payload (base64-encode (compressed-asset-data data))) (esc (code-char 27)))
        (terminal-write viewer (format nil "~C[~D;~DH" esc (1+ (floor y ch)) (1+ (floor x cw))))
        ;; Queue the complete upload together. Hundreds of tiny queue entries
        ;; otherwise force repeated poll/flush cycles for a browser-sized frame.
        (terminal-write viewer
          (with-output-to-string (out nil :element-type 'base-char)
            (loop for offset from 0 below (length payload) by 4096
                  for end = (min (length payload) (+ offset 4096))
                  for more = (if (= end (length payload)) 0 1) do
              (if (zerop offset)
                  (format out "~C_Ga=T,t=d,f=~D,o=z,s=~D,v=~D,i=~D,p=1,C=1,q=2,x=~D,y=~D,w=~D,h=~D,X=~D,Y=~D,m=~D;"
                          esc format width height id sx sy w h (mod x cw) (mod y ch) more)
                  (format out "~C_Gm=~D;" esc more))
              (write-string payload out :start offset :end end)
              (write-char esc out) (write-char #\\ out))))))))

(defun place-outer (viewer id crop cw ch)
  "Move/reclip an already uploaded image without transmitting its pixels."
  (destructuring-bind (x y sx sy w h) crop
    (let ((esc (code-char 27)))
      (terminal-write viewer (format nil "~C[~D;~DH~C_Ga=p,i=~D,p=1,C=1,q=2,x=~D,y=~D,w=~D,h=~D,X=~D,Y=~D;~C\\"
                                     esc (1+ (floor y ch)) (1+ (floor x cw)) esc id
                                     sx sy w h (mod x cw) (mod y ch) esc)))))
(defun fit-label (text width)
  (let ((safe (remove-if (lambda (c) (or (< (char-code c) 32) (> (char-code c) 126))) text)))
    (format nil "~VA" width (subseq safe 0 (min width (length safe))))))
(defun scene-text-rows (cols rows focus panes)
  (let ((output (map 'vector (lambda (ignored) (declare (ignore ignored))
                              (make-string-output-stream)) (make-array rows)))
        (esc (code-char 27)))
    (dotimes (row rows)
      (format (aref output row) "~C[0m~C[~D;1H~A" esc esc (1+ row)
              (make-string cols :initial-element #\Space)))
    (dolist (pane panes)
      (destructuring-bind (id x y width height label status cursor-x cursor-y visible lines placements) pane
        (declare (ignore height cursor-x cursor-y visible placements))
        (format (aref output 0) "~C[1;~DH~C[~Am~A~C[0m" esc (1+ x) esc
                (if (= id focus) "30;46" "37;44")
                (fit-label (format nil " ~D  ~A~A" id label
                                   (if status (format nil " [exit ~D]" status) "")) width) esc)
        (loop for line in lines for row from y do
          (dolist (run line)
            (destructuring-bind (column text attributes) run
              (format (aref output row) "~C[~D;~DH~C[~{~D~^;~}m~A"
                      esc (1+ row) (+ x column 1) esc attributes text))))))
    (when (> (length panes) 1)
      (let ((divider (+ (second (first panes)) (fourth (first panes)))))
        (loop for row from 1 below (1- rows) do
          (format (aref output row) "~C[~D;~DH~C[0;36m│" esc (1+ row) (1+ divider) esc))))
    (format (aref output (1- rows)) "~C[~D;1H~C[0;30;47m~A~C[0m" esc rows esc
            (fit-label " ekko v2 | Ctrl-b: Tab focus  z zoom  s swap  </> divider  d detach  q quit"
                       (1- cols)) esc)
    (map 'vector #'get-output-stream-string output)))

(defun render-scene (viewer)
  (let ((scene (viewer-scene viewer)) (esc (code-char 27))
        (wanted (make-hash-table :test 'equal)))
    (unless scene (return-from render-scene))
    (when (probe-local-transport viewer) (return-from render-scene))
    (destructuring-bind (version cols rows cw ch focus panes) scene
      (unless (= version +wire-version+) (error "Incompatible scene version"))
      (terminal-write viewer (format nil "~C[?2026h~C[?25l" esc esc))
      (dolist (pane panes)
        (dolist (placement (nth 11 pane))
          (let* ((key (list (first pane) (first placement)))
                 (asset (gethash key (viewer-assets viewer)))
                 (crop (when asset (clipped-image pane placement asset cw ch))))
            (when crop (setf (gethash key wanted) (list asset crop))))))
      (maphash (lambda (key old)
                 (let ((new (gethash key wanted)))
                   (unless (and new (= (second old) (first (first new))))
                     (delete-outer viewer (first old))
                     (remhash key (viewer-drawn viewer)))))
               (viewer-drawn viewer))
      ;; Each cached row includes clearing and rendition, so shorter lines
      ;; erase old characters without a full-screen clear that deletes images.
      (let ((current (scene-text-rows cols rows focus panes)))
        (when (or (/= cols (viewer-row-cache-cols viewer))
                  (/= rows (viewer-row-cache-rows viewer)))
          (setf (viewer-row-cache viewer) (make-array 0)))
        (dotimes (index rows)
          (unless (and (< index (length (viewer-row-cache viewer)))
                       (equal (aref current index) (aref (viewer-row-cache viewer) index)))
            (terminal-write viewer (aref current index))))
        (setf (viewer-row-cache viewer) current
              (viewer-row-cache-cols viewer) cols (viewer-row-cache-rows viewer) rows))
      (maphash
       (lambda (key spec)
         (let ((old (gethash key (viewer-drawn viewer)))
               (asset (first spec)) (crop (second spec)))
           (if old
               (unless (and (equal (third old) crop) (= (fourth old) cw) (= (fifth old) ch))
                 (place-outer viewer (first old) crop cw ch)
                 (setf (third old) crop (fourth old) cw (fifth old) ch))
               (let ((id (ekko/client:allocate-outer-id (viewer-ids viewer) (first key) 1 (second key) 0)))
                 (upload-outer viewer id asset crop cw ch)
                 (setf (gethash key (viewer-drawn viewer)) (list id (first asset) crop cw ch))))))
       wanted)
      (let ((active (find focus panes :key #'first)))
        (when (and active (nth 9 active))
          (terminal-write viewer (format nil "~C[~D;~DH~C[?25h" esc
                                         (+ (third active) (nth 8 active) 1)
                                         (+ (second active) (nth 7 active) 1) esc))))
      (terminal-write viewer (format nil "~C[0m~C[?2026l" esc esc))
      (let ((live (make-hash-table :test 'equal)))
        (dolist (pane panes)
          (dolist (placement (nth 11 pane))
            (setf (gethash (list (first pane) (first placement)) live) t)))
        (maphash (lambda (key value) (declare (ignore value))
                   (unless (gethash key live) (remhash key (viewer-assets viewer))))
                 (viewer-assets viewer)))
      (setf (viewer-scene viewer) nil))))
(defun receive-view (viewer packet)
  (case (aref packet 0)
    ((11 13)
     (unless (>= (length packet) 25) (error "Invalid asset"))
     (let ((key (list (u32 packet 1) (u32 packet 5))))
       (setf (gethash key (viewer-pending-assets viewer))
             (list (u32 packet 9) (u32 packet 13) (u32 packet 17) (u32 packet 21)
                   (if (= (aref packet 0) 13)
                       (make-local-asset :path (bytes-text (subseq packet 25))
                                         :size (* (u32 packet 13) (u32 packet 17) (/ (u32 packet 21) 8)))
                       (subseq packet 25))))))
    (12 (when (viewer-awaiting-scene viewer) (error "Overlapping scenes"))
        (setf (viewer-awaiting-scene viewer) t)
        (setf (viewer-scene viewer) (decode-scene (subseq packet 1)))
        (maphash (lambda (key asset) (setf (gethash key (viewer-assets viewer)) asset)) (viewer-pending-assets viewer))
        (clrhash (viewer-pending-assets viewer)))
    (21 (error "~A" (bytes-text (subseq packet 1))))
    (22 (setf (viewer-done viewer) t))
    (otherwise (error "Unexpected server message"))))
(defun input-complete (viewer)
  (let* ((bytes (copy-seq (viewer-input viewer))) (text (map 'string #'code-char bytes))
         (connection (viewer-connection viewer)) (length (length text))
         (final (when (plusp length) (char text (1- length)))))
    (cond
      ((and (> length 3) (string= text (format nil "~C[<" (code-char 27)) :end1 3) (find final "Mm"))
       (send-packet connection 4 bytes))
      ((member text (list (format nil "~C[I" (code-char 27)) (format nil "~C[O" (code-char 27))) :test #'string=)
       (send-packet connection 6 bytes))
      ((string= text (format nil "~C[200~~" (code-char 27)))
       (setf (viewer-paste viewer) t) (send-packet connection 5 bytes))
      ((string= text (format nil "~C[201~~" (code-char 27)))
       (setf (viewer-paste viewer) nil) (send-packet connection 5 bytes))
      ((viewer-paste viewer) (send-packet connection 5 bytes))
      ((and (> length 3) (char= final #\t))
       (let ((args (parameters (subseq text 2 (1- length)))))
         (when (and (= (length args) 3) (= (first args) 6) (<= 1 (second args) 256) (<= 1 (third args) 128))
           (send-size viewer (third args) (second args)))))
      ((and (> length 2) (find final "cnyR")) nil)
      (t (send-packet connection 2 bytes))))
  (setf (fill-pointer (viewer-input viewer)) 0 (viewer-input-state viewer) :ground))
(defun input-feed (viewer bytes count)
  (let ((index 0))
    (loop while (< index count) do
      (let ((byte (aref bytes index)))
        (setf (viewer-input-at viewer) (now))
        (when (> (length (viewer-input viewer)) 4096)
          (setf (viewer-input-state viewer) :discard
                (fill-pointer (viewer-input viewer)) 0))
        (case (viewer-input-state viewer)
          (:ground
           (cond
             ((= byte 27)
              (vector-push-extend byte (viewer-input viewer))
              (setf (viewer-input-state viewer) :escape)
              (incf index))
             ((and (= byte 2) (not (viewer-paste viewer)))
              ;; Keep Ctrl-b separate so the server can recognize it as the
              ;; local prefix key.
              (send-packet (viewer-connection viewer) 2
                           (subseq bytes index (1+ index)))
              (incf index))
             (t
              (let ((end count)
                    (esc (position 27 bytes :start index :end count))
                    (ctrl (and (not (viewer-paste viewer))
                               (position 2 bytes :start index :end count))))
                (when esc (setf end (min end esc)))
                (when ctrl (setf end (min end ctrl)))
                (send-packet (viewer-connection viewer)
                             (if (viewer-paste viewer) 5 2)
                             (subseq bytes index end))
                (setf index end)))))
          (:escape
           (vector-push-extend byte (viewer-input viewer))
           (incf index)
           (case byte
             (91 (setf (viewer-input-state viewer) :csi))
             (95 (setf (viewer-input-state viewer) :apc))
             ((93 80) (setf (viewer-input-state viewer) :discard))
             (79 (setf (viewer-input-state viewer) :ss3))
             (otherwise (input-complete viewer))))
          (:ss3
           (vector-push-extend byte (viewer-input viewer))
           (incf index)
           (input-complete viewer))
          (:csi
           (vector-push-extend byte (viewer-input viewer))
           (incf index)
           (when (<= 64 byte 126) (input-complete viewer)))
          (:apc
           (vector-push-extend byte (viewer-input viewer))
           (incf index)
           (when (and (= byte 92) (> (length (viewer-input viewer)) 1)
                      (= (aref (viewer-input viewer) (- (length (viewer-input viewer)) 2)) 27))
             (unless (viewer-paste viewer)
               (host-graphics-reply viewer (viewer-input viewer)))
             (setf (viewer-input-state viewer) :ground (fill-pointer (viewer-input viewer)) 0)))
          (:discard
           (when (or (= byte 7) (= byte 92))
             (setf (viewer-input-state viewer) :ground
                   (fill-pointer (viewer-input viewer)) 0))
           (incf index)))))))
(defun restore-terminal ()
  (initialize)
  (write-fd 1 (text-bytes *terminal-leave*))
  (uiop:run-program '("stty" "sane") :input :interactive :output :interactive :error-output :interactive)
  0)
(defun attach-session (name &optional connected-fd)
  (initialize)
  (let* ((fd (or connected-fd (checked (connect-local (socket-path name)) "attach")))
         (viewer (make-viewer :connection (make-wire :fd fd) :io (make-wire :fd 1)))
         (buffer (octets 65536)) (last-size 0))
    (dolist (sig (list sb-posix:sigterm sb-posix:sighup))
      (sb-sys:enable-interrupt sig (lambda (&rest arguments) (declare (ignore arguments)) (setf (viewer-done viewer) t))))
    (unwind-protect
         (progn
           (checked (raw 0) "enter terminal raw mode")
           (terminal-write viewer *terminal-enter*) (send-size viewer)
           (loop until (viewer-done viewer) do
             (let ((current (now)))
               (when (>= (- current last-size) 1/5) (send-size viewer) (setf last-size (now)))
               (when (and (eq (viewer-input-state viewer) :escape) (>= (- current (viewer-input-at viewer)) 1/25))
                 (input-complete viewer)))
             (when (and (viewer-probe-deadline viewer) (>= (now) (viewer-probe-deadline viewer)))
               (setf (viewer-transport viewer) :inline (viewer-probe-deadline viewer) nil))
             (when (and (viewer-awaiting-scene viewer) (null (viewer-scene viewer))
                        (null (wire-queue (viewer-io viewer)))
                        (zerop (hash-table-count (viewer-uploads viewer))))
               (send-packet (viewer-connection viewer) 14)
               (setf (viewer-awaiting-scene viewer) nil))
             (when (null (wire-queue (viewer-io viewer))) (render-scene viewer))
             ;; Poll until the next scheduled maintenance task.  Terminal
             ;; escape disambiguation remains a 40ms deadline, while resize
             ;; checks remain at 200ms; active descriptors still wake us early.
             (let* ((current (now))
                    (deadline (+ last-size 1/5)))
               (when (eq (viewer-input-state viewer) :escape)
                 (setf deadline (min deadline (+ (viewer-input-at viewer) 1/25))))
               (dolist (event (poll-fds (append (list (cons 0 1) (cons fd (wire-events (viewer-connection viewer))))
                                                (when (wire-queue (viewer-io viewer)) (list (cons 1 4))))
                                      (max 0 (ceiling (* 1000 (- deadline current))))))
               (let ((flags (cdr event)))
                 (cond ((= (car event) 0)
                        (let ((n (read-fd 0 buffer)))
                          (cond ((plusp n) (input-feed viewer buffer n)) ((not (member n '(-11 -4))) (setf (viewer-done viewer) t)))))
                       ((= (car event) 1) (flush-wire (viewer-io viewer)))
                       ((= (car event) fd)
                        (when (logtest flags 4) (flush-wire (viewer-connection viewer)))
                        (when (logtest flags 25)
                          (handler-case (dolist (packet (receive-packets (viewer-connection viewer) buffer)) (receive-view viewer packet))
                            (error (e) (if (search "disconnected" (princ-to-string e)) (setf (viewer-done viewer) t) (error e))))))))))))
      (ignore-errors
        (let ((deadline (+ (now) 1)))
          (loop while (and (wire-queue (viewer-io viewer)) (< (now) deadline)) do
            (flush-wire (viewer-io viewer)) (when (wire-queue (viewer-io viewer)) (poll-fds '((1 . 4)) 10))))
        (maphash (lambda (key value) (declare (ignore key)) (delete-outer viewer (first value))) (viewer-drawn viewer))
        (terminal-write viewer *terminal-leave*)
        (loop repeat 20 while (wire-queue (viewer-io viewer)) do (flush-wire (viewer-io viewer)) (poll-fds '((1 . 4)) 10)))
      (restore) (close-wire (viewer-connection viewer)) (ekko/client:attachment-teardown (viewer-ids viewer))))
  0)
(defun control-session (name command)
  (initialize)
  (let* ((wire (make-wire :fd (checked (connect-local (socket-path name)) "connect")))
         (buffer (octets 65536)) (deadline (+ (now) 5)))
    (unwind-protect
         (progn
           (send-packet wire 3 (text-bytes command))
           (loop while (< (now) deadline) do
             (dolist (event (poll-fds (list (cons (wire-fd wire) (wire-events wire))) 100))
               (when (logtest (cdr event) 4) (flush-wire wire))
               (when (logtest (cdr event) 25)
                 (dolist (packet (receive-packets wire buffer))
                   (when (= (aref packet 0) 20)
                     (when (string= command "status") (write-string (bytes-text (subseq packet 1))))
                   (return-from control-session 0))))))
           (error "Control request timed out"))
      (close-wire wire))))
(defun run-session (name commands)
  (initialize)
  (let* ((path (socket-path name)) (fd (connect-local path)))
    (when (minusp fd)
      (unless (member fd '(-2 -111)) (checked fd "connect"))
      (let* ((binary (car sb-ext:*posix-argv*)) (log (concatenate 'string path ".log"))
             (process (sb-ext:run-program binary
                                          (append (list "--serve" name) (first commands)
                                                  (when (second commands) (cons ":::" (second commands))))
                                          :wait nil :input nil :output log :error :output :if-output-exists :append))
             (deadline (+ (now) 10)))
        (loop while (and (minusp fd) (< (now) deadline)) do
          (unless (sb-ext:process-alive-p process) (error "Session failed to start; see ~A" log))
          (poll-fds nil 30) (setf fd (connect-local path)))
        (when (minusp fd) (error "Session startup timed out; see ~A" log))))
    (attach-session name fd)))
