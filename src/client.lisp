(in-package #:ekko/runtime)

(defparameter *terminal-enter*
  (format nil "~C[?1049h~C[?25l~C[?1003h~C[?1006h~C[?1016h~C[?1004h~C[?2004h~C[>1u~C[14t~C[16t"
          #1=(code-char 27) #1# #1# #1# #1# #1# #1# #1# #1# #1#))
(defparameter *terminal-leave*
  (format nil "~C[?2026l~C[<u~C[?1003l~C[?1006l~C[?1016l~C[?1004l~C[?2004l~C[0m~C[?25h~C[?1049l"
          #1=(code-char 27) #1# #1# #1# #1# #1# #1# #1# #1# #1#))
(defstruct viewer io connection scene exit-text (assets (make-hash-table :test 'equal))
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
  (input-read-bytes (octets 0)) (input-generation 0)
  (binding-generation 0) (binding-state :unbound) discard-paste cancel-paste-marker (host-focused t)
  (input-at 0) (paste nil) (done nil) size reported-cell-size (cw 8) (ch 16))
(defun terminal-write (viewer text) (queue-bytes (viewer-io viewer) (text-bytes text)))
(defun terminal-viewport ()
  "Return a validated viewport for a detached server, or NIL without a tty."
  (handler-case
      (destructuring-bind (cols rows pixel-cols pixel-rows) (terminal-size 0)
        (when (and (plusp cols) (plusp rows))
          (let ((cw (if (plusp pixel-cols) (floor pixel-cols cols) 8))
                (ch (if (plusp pixel-rows) (floor pixel-rows rows) 16)))
            (list (max 5 (min 500 cols)) (max 4 (min 300 rows))
                  (max 1 (min 128 cw)) (max 1 (min 256 ch))))))
    (error () nil)))
(defun send-size (viewer &optional queried-cw queried-ch)
  (let* ((size (terminal-size 0)) (cols (first size)) (rows (second size))
         (cw (or queried-cw (second (viewer-reported-cell-size viewer))
                 (and (plusp cols) (plusp (third size)) (floor (third size) cols)) (viewer-cw viewer)))
         (ch (or queried-ch (third (viewer-reported-cell-size viewer))
                 (and (plusp rows) (plusp (fourth size)) (floor (fourth size) rows)) (viewer-ch viewer))))
    (setf cols (max 5 (min 500 cols)) rows (max 4 (min 300 rows))
          cw (max 1 (min 128 cw)) ch (max 1 (min 256 ch)))
    (when (or (not (equal size (viewer-size viewer))) (/= cw (viewer-cw viewer)) (/= ch (viewer-ch viewer)))
      (setf (viewer-size viewer) size (viewer-cw viewer) cw (viewer-ch viewer) ch)
      (send-packet (viewer-connection viewer) 17 (integers (list cols rows cw ch))))))
(defun report-cell-size (viewer width height source)
  ;; A direct character-cell answer takes precedence over a text-area estimate.
  ;; Physical viewport changes precede the separate observation: receiving a
  ;; measurement does not itself impose an application PTY resize policy.
  (when (and (<= 1 width 128) (<= 1 height 256)
             (not (and (eq source :area)
                       (eq (first (viewer-reported-cell-size viewer)) :cell))))
    (send-size viewer width height)
    (unless (equal (list source width height) (viewer-reported-cell-size viewer))
      (setf (viewer-reported-cell-size viewer) (list source width height))
      (send-packet (viewer-connection viewer) 15 (integers (list width height))))))
(defun clipped-image (pane placement asset cw ch &optional viewport)
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
          (when (and cut viewport)
            (setf cut (ekko/scene:rect-intersection cut viewport)))
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
(defun render-decoration (stream cols rows decoration)
  (destructuring-bind (x y text sgr) decoration
    (when (and (integerp x) (integerp y) (stringp text)
               (<= 0 x) (< x cols) (<= 0 y) (< y rows))
      ;; The server clips spans at cell boundaries and never publishes a
      ;; decoration over app content. Keep this renderer deliberately small:
      ;; it only emits the already validated, published cells.
      (format stream "~C[~D;~DH~C[~{~D~^;~}m~A~C[0m"
              (code-char 27) (1+ y) (1+ x) (code-char 27) sgr text (code-char 27)))))
(defun scene-content-rect (cols rows metadata &optional (cw 1) (ch 1))
  "Return the content viewport in cells, or pixels when cell metrics are given."
  (destructuring-bind (top right bottom left)
      (effective-insets cols rows (getf metadata :viewport-insets '(0 0 0 0)))
    (ekko/scene:make-rect (* left cw) (* top ch)
                         (* (- cols left right) cw) (* (- rows top bottom) ch))))

(defun clip-text-run (text x left right)
  "Clip a run on cell boundaries; combining marks stay with an accepted base."
  (let ((cursor x) (start nil) (accepted nil) (characters nil))
    (loop for character across text for width = (ekko/text:display-width character) do
      (cond ((zerop width) (when accepted (push character characters)))
            ((and (<= left cursor) (<= (+ cursor width) right))
             (unless start (setf start cursor))
             (setf accepted t)
             (push character characters))
            (t (setf accepted nil)))
      (incf cursor width))
    (values start (when start (coerce (nreverse characters) 'string)))))

(defun scene-text-rows (cols rows focus panes &optional metadata)
  (declare (ignore focus))
  (let ((output (map 'vector (lambda (ignored) (declare (ignore ignored))
                               (make-string-output-stream)) (make-array rows)))
        (viewport (scene-content-rect cols rows metadata))
        (esc (code-char 27)))
    (dotimes (row rows)
      (format (aref output row) "~C[0m~C[~D;1H~A" esc esc (1+ row)
              (make-string cols :initial-element #\Space)))
    (dolist (pane panes)
      (destructuring-bind (id x y width height label status cursor-x cursor-y visible lines placements outer) pane
        (declare (ignore id label status cursor-x cursor-y visible placements))
        (let ((background (ekko/scene:rect-intersection
                           (apply #'ekko/scene:make-rect outer) viewport))
              (content (ekko/scene:rect-intersection
                        (ekko/scene:make-rect x y width height) viewport)))
          (when background
            (loop for row from (ekko/scene:rect-y background)
                  below (ekko/scene:rect-bottom background) do
              (format (aref output row) "~C[0m~C[~D;~DH~A" esc esc (1+ row)
                      (1+ (ekko/scene:rect-x background))
                      (make-string (ekko/scene:rect-width background) :initial-element #\Space))))
          (when content
            (loop for line in lines for row from y
                  when (and (<= (ekko/scene:rect-y content) row)
                            (< row (ekko/scene:rect-bottom content))) do
              (dolist (run line)
                (destructuring-bind (column text attributes) run
                  (multiple-value-bind (start clipped)
                      (clip-text-run text (+ x column) (ekko/scene:rect-x content)
                                     (ekko/scene:rect-right content))
                    (when start
                      (format (aref output row) "~C[~D;~DH~C[~{~D~^;~}m~A"
                              esc (1+ row) (1+ start) esc attributes clipped))))))))))
    ;; Chrome policy is supplied by Lisp hooks as bounded spans. The server
    ;; clips these before publication, and the client merely places them.
    (dolist (decoration (append (getf metadata :decorations) (getf metadata :overlays)))
      (let ((y (second decoration)))
        (when (and (integerp y) (<= 0 y) (< y rows))
          (render-decoration (aref output y) cols rows decoration))))
    (map 'vector #'get-output-stream-string output)))

(defun overlay-image-crops (crop overlays cw ch &optional occlusions)
  "Subtract opaque cell spans from a pixel crop, preserving source offsets."
  (when crop
    (destructuring-bind (x y sx sy w h) crop
      (let ((pieces (list (ekko/scene:make-rect x y w h))))
        (dolist (span overlays)
          (destructuring-bind (ox oy text sgr) span
            (declare (ignore sgr))
            (let ((width (ekko/text:display-width text)))
              (when (plusp width)
                (let ((cut (ekko/scene:make-rect (* ox cw) (* oy ch) (* width cw) ch)))
                  (setf pieces (mapcan (lambda (piece) (ekko/scene:rect-subtract piece cut)) pieces)))))))
        (dolist (rect occlusions)
          (destructuring-bind (ox oy ow oh) rect
            (let ((cut (ekko/scene:make-rect (* ox cw) (* oy ch) (* ow cw) (* oh ch))))
              (setf pieces (mapcan (lambda (piece) (ekko/scene:rect-subtract piece cut)) pieces)))))
        (loop for piece in pieces collect
          (let ((px (ekko/scene:rect-x piece)) (py (ekko/scene:rect-y piece)))
            (list px py (+ sx (- px x)) (+ sy (- py y))
                  (ekko/scene:rect-width piece) (ekko/scene:rect-height piece))))))))

(defun render-scene (viewer)
  (let ((scene (viewer-scene viewer)) (esc (code-char 27))
        (wanted (make-hash-table :test 'equal)))
    (unless scene (return-from render-scene))
    (when (probe-local-transport viewer) (return-from render-scene))
    (destructuring-bind (version cols rows cw ch focus panes &optional metadata) scene
      (unless (= version +wire-version+) (error "Incompatible scene version"))
      (terminal-write viewer (format nil "~C[?2026h~C[?25l" esc esc))
      (dolist (pane panes)
        (dolist (placement (nth 11 pane))
          (let* ((key (list (first pane) (first placement)))
                 (asset (gethash key (viewer-assets viewer)))
                 (crop (when asset (clipped-image pane placement asset cw ch
                                                  (scene-content-rect cols rows metadata cw ch)))))
            (loop for fragment in (overlay-image-crops crop (getf metadata :overlays) cw ch (mapcar (lambda (p) (nth 12 p)) (rest (member pane panes))))
                  for index from 0 do
                    (setf (gethash (append key (list index)) wanted) (list asset fragment))))))
      (maphash (lambda (key old)
                 (let ((new (gethash key wanted)))
                   (unless (and new (= (second old) (first (first new))))
                     (delete-outer viewer (first old))
                     (remhash key (viewer-drawn viewer)))))
               (viewer-drawn viewer))
      ;; Each cached row includes clearing and rendition, so shorter lines
      ;; erase old characters without a full-screen clear that deletes images.
      (let ((current (scene-text-rows cols rows focus panes metadata)))
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
               (let ((id (ekko/client:allocate-outer-id (viewer-ids viewer) (first key) 1 (second key) 0 (third key))))
                 (upload-outer viewer id asset crop cw ch)
                 (setf (gethash key (viewer-drawn viewer)) (list id (first asset) crop cw ch))))))
       wanted)
      (let* ((active (find focus panes :key #'first))
             (cx (when active (+ (second active) (nth 7 active))))
             (cy (when active (+ (third active) (nth 8 active))))
             (viewport (scene-content-rect cols rows metadata)))
        (when (and active (nth 9 active) (null (getf metadata :overlays))
                   (<= (ekko/scene:rect-x viewport) cx) (< cx (ekko/scene:rect-right viewport))
                   (<= (ekko/scene:rect-y viewport) cy) (< cy (ekko/scene:rect-bottom viewport))
                   (not (some (lambda (p)
                                (destructuring-bind (x y w h) (nth 12 p)
                                  (and (<= x cx) (< cx (+ x w)) (<= y cy) (< cy (+ y h)))))
                              (rest (member active panes)))))
          (terminal-write viewer (format nil "~C[~D;~DH~C[?25h" esc (1+ cy) (1+ cx) esc))))
      (terminal-write viewer (format nil "~C[0m~C[?2026l" esc esc))
      (let ((live (make-hash-table :test 'equal)))
        (dolist (pane panes)
          (dolist (placement (nth 11 pane))
            (setf (gethash (list (first pane) (first placement)) live) t)))
        (maphash (lambda (key value) (declare (ignore value))
                   (unless (gethash key live) (remhash key (viewer-assets viewer))))
                 (viewer-assets viewer)))
      (setf (viewer-scene viewer) nil))))
(defun paste-input-p (viewer)
  (or (viewer-paste viewer) (viewer-discard-paste viewer)))
(defun pending-paste-marker-p (viewer)
  (let ((input (viewer-input viewer)))
    (and (member (viewer-input-state viewer) '(:escape :csi))
         (<= 1 (length input) 5)
         (some (lambda (marker) (not (mismatch input marker :end2 (length input))))
               '(#(27 91 50 48 48 126) #(27 91 50 48 49 126))))))
(defun clear-client-input (viewer)
  ;; Keep only cancellation knowledge, never an old original-read transaction.
  ;; A marker prefix can exist before the drain or at its EAGAIN boundary.
  (setf (viewer-cancel-paste-marker viewer)
        (or (viewer-cancel-paste-marker viewer) (pending-paste-marker-p viewer))
        (viewer-input-read-bytes viewer) (octets 0))
  (unless (viewer-cancel-paste-marker viewer)
    (setf (viewer-input-state viewer) :ground (fill-pointer (viewer-input viewer)) 0
          (viewer-input-at viewer) 0)))
(defun drain-binding-input (viewer)
  ;; Raw stdin is nonblocking. Discard a bounded old backlog, but parse paste
  ;; terminators so an old paste tail cannot become keys in the new binding.
  (let ((buffer (octets 65536)))
    (loop repeat 16 for count = (read-fd 0 buffer) do
      (cond ((plusp count)
             (setf (viewer-input-read-bytes viewer) (octets 0))
             (input-feed viewer buffer count))
            ((= count -11) (return-from drain-binding-input))
            ((= count -4))
            ((zerop count) (setf (viewer-done viewer) t) (return-from drain-binding-input))
            (t (checked count "discard old terminal input"))))
    (error "Terminal input did not quiesce at binding change")))
(defun finish-input-binding (viewer)
  (when (and (eq (viewer-binding-state viewer) :accepting)
             (not (viewer-discard-paste viewer)) (not (viewer-cancel-paste-marker viewer)))
    (setf (viewer-binding-state viewer) :ready)
    (send-packet (viewer-connection viewer) 25 (integers (list (viewer-binding-generation viewer))))
    ;; Physical connection observations survive a session switch. They are not
    ;; an old stdin transaction, so they use the newly accepted generation.
    (send-packet (viewer-connection viewer) 6
      (concatenate '(vector (unsigned-byte 8)) (integers (list (viewer-binding-generation viewer)))
                   (text-bytes (format nil "~C[~A" #\Esc (if (viewer-host-focused viewer) "I" "O")))))
    (when (viewer-reported-cell-size viewer)
      (send-packet (viewer-connection viewer) 15
                   (integers (rest (viewer-reported-cell-size viewer)))))
    (terminal-write viewer (format nil "~C[14t~C[16t" #\Esc #\Esc))))
(defun accept-input-binding (viewer data)
  (unless (= (length data) 4) (error "Malformed input binding notice"))
  (let ((generation (u32 data 0)))
    (unless (and (plusp generation) (= generation (1+ (viewer-binding-generation viewer))))
      (error "Unexpected input binding generation"))
    (setf (viewer-binding-state viewer) :draining
          (viewer-discard-paste viewer) (paste-input-p viewer)
          (viewer-paste viewer) nil)
    (clear-client-input viewer)
    (drain-binding-input viewer)
    (setf (viewer-discard-paste viewer) (paste-input-p viewer)
          (viewer-paste viewer) nil)
    (clear-client-input viewer)
    (setf (viewer-binding-generation viewer) generation (viewer-input-generation viewer) generation
          (viewer-binding-state viewer) :accepting)
    ;; A paste can continue after the old OS backlog drains. Delay acceptance
    ;; until its terminator, so no new graphics replies interleave a discarded
    ;; old paste and no old tail becomes an application key.
    (finish-input-binding viewer)))
(defun receive-view (viewer packet)
  (case (aref packet 0)
    (24 (accept-input-binding viewer (subseq packet 1)))
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
        (let* ((scene (decode-scene (subseq packet 1)))
               (exit-text (getf (nth 7 scene) :exit-text)))
          (unless (or (null exit-text)
                      (and (stringp exit-text) (<= (length exit-text) 512)
                           (valid-decoration-text-p exit-text)))
            (error "Invalid viewer exit text"))
          (setf (viewer-awaiting-scene viewer) t
                (viewer-scene viewer) scene
                (viewer-exit-text viewer) exit-text))
        (maphash (lambda (key asset) (setf (gethash key (viewer-assets viewer)) asset)) (viewer-pending-assets viewer))
        (clrhash (viewer-pending-assets viewer)))
    (23 (when (> (1- (length packet)) (* 1024 1024)) (error "Clipboard exceeds 1 MiB"))
        (terminal-write viewer (format nil "~C]52;c;~A~C\\" #\Esc
                                       (base64-encode (subseq packet 1)) #\Esc)))
    (21 (error "~A" (bytes-text (subseq packet 1))))
    (22 (setf (viewer-done viewer) t))
    (otherwise (error "Unexpected server message"))))
(defun send-input-packet (viewer kind bytes)
  ;; Every event from an original read keeps that read's captured generation.
  ;; Queued old wire bytes need not be rewritten; the daemon drops their prefix.
  (when (or (not (eq (viewer-binding-state viewer) :ready)) (zerop (viewer-input-generation viewer))
            (viewer-discard-paste viewer))
    (setf (viewer-input-read-bytes viewer) (octets 0))
    (return-from send-input-packet))
  (let ((generation (integers (list (viewer-input-generation viewer)))))
    (when (plusp (length (viewer-input-read-bytes viewer)))
      (send-packet (viewer-connection viewer) 16
                   (concatenate '(vector (unsigned-byte 8)) generation (viewer-input-read-bytes viewer)))
      (setf (viewer-input-read-bytes viewer) (octets 0)))
    (send-packet (viewer-connection viewer) kind
                 (concatenate '(vector (unsigned-byte 8)) generation bytes))))
(defun finish-client-input (viewer)
  (when (viewer-cancel-paste-marker viewer)
    (setf (viewer-input-read-bytes viewer) (octets 0)))
  (setf (fill-pointer (viewer-input viewer)) 0 (viewer-input-state viewer) :ground
        (viewer-cancel-paste-marker viewer) nil)
  (finish-input-binding viewer))
(defun input-complete (viewer)
  (let* ((bytes (copy-seq (viewer-input viewer))) (text (map 'string #'code-char bytes))
         (length (length text))
         (final (when (plusp length) (char text (1- length)))))
    (cond
      ((string= text (format nil "~C[200~~" (code-char 27)))
       (if (viewer-cancel-paste-marker viewer)
           (setf (viewer-discard-paste viewer) t (viewer-paste viewer) nil)
           (setf (viewer-paste viewer) t))
       (send-input-packet viewer 5 bytes))
      ((string= text (format nil "~C[201~~" (code-char 27)))
       (setf (viewer-paste viewer) nil)
       (if (viewer-discard-paste viewer)
           (setf (viewer-discard-paste viewer) nil (viewer-input-read-bytes viewer) (octets 0))
           (send-input-packet viewer 5 bytes)))
      ((viewer-cancel-paste-marker viewer)
       ;; A candidate that instead completes another CSI is still old input.
       (setf (viewer-input-read-bytes viewer) (octets 0)))
      ((paste-input-p viewer) (send-input-packet viewer 5 bytes))
      ((and (> length 3) (string= text (format nil "~C[<" (code-char 27)) :end1 3) (find final "Mm"))
       (send-input-packet viewer 4 bytes))
      ((member text (list (format nil "~C[I" (code-char 27)) (format nil "~C[O" (code-char 27))) :test #'string=)
       (setf (viewer-host-focused viewer) (string= text (format nil "~C[I" #\Esc)))
       (send-input-packet viewer 6 bytes))
      ((and (> length 3) (char= final #\t))
       (let ((args (parameters (subseq text 2 (1- length)))))
         (when (= (length args) 3)
           (case (first args)
             (6 (report-cell-size viewer (third args) (second args) :cell))
             (4 (let ((size (terminal-size 0)))
                  (when (and (plusp (first size)) (plusp (second size)))
                    (report-cell-size viewer (floor (third args) (first size))
                                      (floor (second args) (second size)) :area))))))))
      ((and (> length 2) (find final "cnyR")) nil)
      (t (send-input-packet viewer 2 bytes))))
  (finish-client-input viewer))
(defun input-feed (viewer bytes count)
  (when (zerop (length (viewer-input-read-bytes viewer)))
    (setf (viewer-input-generation viewer) (viewer-binding-generation viewer)))
  (when (> (+ (length (viewer-input-read-bytes viewer)) count) 65536)
    (error "Pending original input read exceeds 64 KiB"))
  (setf (viewer-input-read-bytes viewer)
        (concatenate '(vector (unsigned-byte 8))
                     (viewer-input-read-bytes viewer) (subseq bytes 0 count)))
  (let ((index 0))
    (loop while (< index count) do
      (let ((byte (aref bytes index)))
        (setf (viewer-input-at viewer) (now))
        ;; An interleaved escape makes an old marker's remaining bytes ambiguous.
        ;; Fail this viewer rather than retagging a host-event tail as new input.
        (when (and (viewer-cancel-paste-marker viewer) (= byte 27)
                   (member (viewer-input-state viewer) '(:escape :csi :ss3)))
          (error "Interrupted cancelled paste marker"))
        (when (> (length (viewer-input viewer)) 4096)
          (when (viewer-cancel-paste-marker viewer)
            (error "Cancelled input sequence exceeds 4 KiB"))
          (setf (viewer-input-state viewer) :discard
                (fill-pointer (viewer-input viewer)) 0))
        (case (viewer-input-state viewer)
          (:ground
           (cond
             ((= byte 27)
              (vector-push-extend byte (viewer-input viewer))
              (setf (viewer-input-state viewer) :escape)
              (incf index))
             ((and (= byte 2) (not (paste-input-p viewer)))
              ;; Keep Ctrl-b separate so the server can recognize it as the
              ;; local prefix key.
              (send-input-packet viewer 2
                           (subseq bytes index (1+ index)))
              (incf index))
             (t
              (let ((end count)
                    (esc (position 27 bytes :start index :end count))
                    (ctrl (and (not (paste-input-p viewer))
                               (position 2 bytes :start index :end count))))
                (when esc (setf end (min end esc)))
                (when ctrl (setf end (min end ctrl)))
                (send-input-packet viewer
                             (if (paste-input-p viewer) 5 2)
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
             (unless (paste-input-p viewer)
               (host-graphics-reply viewer (viewer-input viewer)))
             (finish-client-input viewer)))
          (:discard
           (when (or (= byte 7) (= byte 92))
             (finish-client-input viewer))
           (incf index))))))
  ;; A completely filtered terminal reply must not become a later key's read.
  (when (eq (viewer-input-state viewer) :ground)
    (setf (viewer-input-read-bytes viewer) (octets 0))))
(defun restore-terminal ()
  (initialize)
  (write-fd 1 (text-bytes *terminal-leave*))
  (uiop:run-program '("stty" "sane") :input :interactive :output :interactive :error-output :interactive)
  0)
(defun attach-session (name &optional connected-fd view-id)
  (initialize)
  (checked-name name)
  (let* ((viewport (or (terminal-viewport) (error "attach requires a terminal")))
         (fd (or connected-fd (checked (connect-local (socket-path)) "attach")))
         (viewer (make-viewer :connection (make-wire :fd fd) :io (make-wire :fd 1)))
         (buffer (octets 65536)) (last-size 0))
    (dolist (sig (list sb-posix:sigterm sb-posix:sighup))
      (sb-sys:enable-interrupt sig (lambda (&rest arguments) (declare (ignore arguments)) (setf (viewer-done viewer) t))))
    (unwind-protect
         (progn
           (send-route (viewer-connection viewer) name view-id)
           (send-packet (viewer-connection viewer) 1 (integers (cons +wire-version+ viewport)))
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
        (when (viewer-exit-text viewer)
          (terminal-write viewer (format nil "~A~C~C" (viewer-exit-text viewer)
                                         #\Return #\Newline)))
        (loop repeat 20 while (wire-queue (viewer-io viewer)) do (flush-wire (viewer-io viewer)) (poll-fds '((1 . 4)) 10)))
      (restore) (close-wire (viewer-connection viewer)) (ekko/client:attachment-teardown (viewer-ids viewer))))
  0)
(defun wait-control-result (wire &optional (timeout 10))
  (let ((buffer (octets 65536)) (deadline (+ (now) timeout)))
    (loop while (< (now) deadline) do
      (dolist (event (poll-fds (list (cons (wire-fd wire) (wire-events wire))) 100))
        (when (logtest (cdr event) 4) (flush-wire wire))
        (when (logtest (cdr event) 25)
          (dolist (packet (receive-packets wire buffer))
            (case (aref packet 0)
              (21 (error "~A" (bytes-text (subseq packet 1))))
              (20 (return-from wait-control-result (bytes-text (subseq packet 1))))
              (otherwise (error "Unexpected control reply")))))))
    (error "Control request timed out")))

(defun control-session (name command &key arguments view-id)
  (initialize)
  (let ((wire (make-wire :fd (checked (connect-local (socket-path)) "connect"))))
    (unwind-protect
         (progn
           (send-route wire name view-id)
           (cond ((equal command "switch")
                  (send-packet wire 9 (text-bytes (first arguments))))
                 (arguments
                  (send-packet wire 7
                    (text-bytes (with-output-to-string (out)
                                  (loop for value in arguments for first = t then nil do
                                    (unless first (write-char #\Null out)) (write-string value out))))))
                 (t (send-packet wire 3 (text-bytes command))))
           (let ((result (wait-control-result wire)))
             (when (member command '("list" "status" "inspect" "buffer") :test #'equal)
               (write-string result)))
           0)
      (close-wire wire))))

(defun ensure-daemon ()
  (let* ((path (socket-path)) (fd (connect-local path)))
    (when (minusp fd)
      (unless (member fd '(-2 -111)) (checked fd "connect"))
      (let ((log (concatenate 'string path ".log")) (deadline (+ (now) 10)))
        ;; A losing starter may exit before the lock winner listens.
        (sb-ext:run-program "/proc/self/exe" (list "--instance" *instance* "--serve")
                            :wait nil :input nil :output log :error :output :if-output-exists :append)
        (loop while (and (minusp fd) (< (now) deadline)) do
          (poll-fds nil 30)
          (setf fd (connect-local path)))
        (when (minusp fd) (error "Daemon startup timed out; see ~A" log))))
    fd))

(defun creation-config-path ()
  (let ((path (config-path)))
    (cond ((probe-file path) (namestring (truename path)))
          ((uiop:getenv "EKKO_CONFIG") (error "Configuration does not exist: ~A" path))
          (t nil))))

(defun run-session (name commands &key detached)
  (initialize)
  (checked-name name)
  (let* ((request (list commands (terminal-viewport) (namestring (truename (uiop:getcwd)))
                        (sb-ext:posix-environ) (creation-config-path)))
         (wire (make-wire :fd (ensure-daemon))))
    (unwind-protect
         (progn
           (send-route wire name)
           (send-packet wire 8 (encode-scene request))
           (wait-control-result wire))
      (close-wire wire)))
  (if detached 0 (attach-session name)))
