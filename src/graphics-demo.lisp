(defpackage #:ekko/graphics-demo
  (:use #:cl)
  (:export #:make-demo-transactions #:make-checkerboard-transactions
           #:make-native-checkerboard-transactions
           #:replay #:checkerboard-pixel #:executable-main))
(in-package #:ekko/graphics-demo)
(defconstant +escape+ 27)
(defun ascii-octets (text) (map '(vector (unsigned-byte 8)) #'char-code text))
(defun kitty-transaction (outer-id x-cell y-cell payload &key (width 1) (height 1)
                          (cells-width 3) (cells-height 1) native-p (x-offset 0) (y-offset 0))
  (concatenate '(vector (unsigned-byte 8))
    (ascii-octets (format nil "~C[~D;~DH~C_Ga=T,f=24,s=~D,v=~D,m=0,q=2,i=~D,p=1~A;~A~C\\"
                          (code-char +escape+) (1+ y-cell) (1+ x-cell)
                          (code-char +escape+) width height outer-id
                          (if native-p (format nil ",X=~D,Y=~D" x-offset y-offset)
                              (format nil ",c=~D,r=~D" cells-width cells-height))
                          payload (code-char +escape+)))))
(defun pane-cell (destination source pane)
  (let ((fragment (first (ekko/scene:clip-placement destination source pane pane pane))))
    (unless fragment (error "Synthetic placement was fully clipped"))
    (let ((rect (ekko/scene:mapped-fragment-destination fragment)))
      (unless (and (integerp (/ (ekko/scene:rect-x rect) 8))
                   (integerp (/ (ekko/scene:rect-y rect) 16))
                   (= (ekko/scene:rect-width rect) 24)
                   (= (ekko/scene:rect-height rect) 16))
        (error "Fixture requires an aligned 3 by 1 cell destination"))
      (values (floor (ekko/scene:rect-x rect) 8) (floor (ekko/scene:rect-y rect) 16)))))
(defun make-demo-transactions (client)
  (let ((source (ekko/scene:make-rect 0 0 1 1))
        (pane-a (ekko/scene:make-rect 0 0 24 16))
        (pane-b (ekko/scene:make-rect 32 0 24 16)))
    (multiple-value-bind (ax ay) (pane-cell pane-a source pane-a)
      (multiple-value-bind (bx by) (pane-cell pane-b source pane-b)
        (let ((transactions
                (list (kitty-transaction (ekko/client:allocate-outer-id client 1 1 7 0) ax ay "/wAA")
                      (kitty-transaction (ekko/client:allocate-outer-id client 2 1 7 0) bx by "AAD/"))))
          (dolist (transaction transactions) (ekko/client:enqueue-transaction client transaction))
          transactions)))))

(defun checkerboard-pixel (x y)
  "Return the RGB pixel used by the deterministic 5 by 4 fixture image."
  (if (evenp (+ x y)) '(255 220 40) '(35 90 220)))

(defun checkerboard-payload (source)
  "Return the fixed Base64 payload for either visible 1 by 2 source crop.
The bytes are RGB pixels (yellow, blue): ff dc 28 23 5a dc."
  (unless (and (= (ekko/scene:rect-width source) 1)
               (= (ekko/scene:rect-height source) 2)
               (member (ekko/scene:rect-x source) '(1 3))
               (= (ekko/scene:rect-y source) 1)
               (equal (checkerboard-pixel (ekko/scene:rect-x source) 1)
                      '(255 220 40))
               (equal (checkerboard-pixel (ekko/scene:rect-x source) 2)
                      '(35 90 220)))
    (error "Unexpected checkerboard crop: ~S" source))
  "/9woI1rc")

;; Fixed native RGB rows generated from the formula above (no runtime codec).
(defparameter *row-even* "/9woI1rc/9woI1rc/9woI1rc/9woI1rc/9woI1rc/9woI1rc/9woI1rc/9woI1rc/9woI1rc/9woI1rc/9woI1rc/9woI1rc")
(defparameter *row-odd* "I1rc/9woI1rc/9woI1rc/9woI1rc/9woI1rc/9woI1rc/9woI1rc/9woI1rc/9woI1rc/9woI1rc/9woI1rc/9woI1rc/9wo")
(defparameter *row9-even* "/9woI1rc/9woI1rc/9woI1rc/9woI1rc/9wo")
(defparameter *row9-odd* "I1rc/9woI1rc/9woI1rc/9woI1rc/9woI1rc")
(defun native-payload (width height x y)
  (unless (and (member width '(24 9)) (member height '(3 4 9))
               (typep x '(integer 0 25)) (typep y '(integer 0 17))
               (<= (+ x width) 26) (<= (+ y height) 18))
    (error "Unsupported fixed native crop ~Sx~S" width height))
  (apply #'concatenate 'string
         (loop for row below height
               for parity = (mod (+ row y x) 2)
               collect (cond ((= width 24) (if (zerop parity) *row-even* *row-odd*))
                             (t (if (zerop parity) *row9-even* *row9-odd*))))))

(defun make-checkerboard-transactions (client)
  "Queue cropped fragments of one oversized checkerboard in each pane.
The placement is 40 by 32 pixels at (-8,-8), so clipping crosses every
edge.  An opaque 8 by 16 overlay in the middle leaves two fragments." 
  (let ((source (ekko/scene:make-rect 0 0 5 4))
        (transactions '()))
    (dolist (pane (list (ekko/scene:make-rect 0 0 24 16)
                        (ekko/scene:make-rect 32 0 24 16)))
      (let ((destination (ekko/scene:make-rect (+ (ekko/scene:rect-x pane) -8) -8 40 32))
            (overlay (ekko/scene:make-rect (+ (ekko/scene:rect-x pane) 8) 0 8 16))
            (fragment-id 0))
      (dolist (fragment (ekko/scene:clip-placement destination source pane pane pane
                                                     (list overlay)))
        (incf fragment-id)
        (let* ((rect (ekko/scene:mapped-fragment-destination fragment))
               (src (ekko/scene:mapped-fragment-source fragment))
               (dx (ekko/scene:rect-width rect))
               (dy (ekko/scene:rect-height rect)))
          (unless (and (integerp (/ (ekko/scene:rect-x rect) 8))
                       (integerp (/ (ekko/scene:rect-y rect) 16))
                       (integerp dx) (integerp dy)
                       (zerop (mod dx 8)) (zerop (mod dy 16)))
            (error "Checkerboard fragment is not cell aligned: ~S" rect))
          (let ((transaction
                  (kitty-transaction
                   (ekko/client:allocate-outer-id client
                                                   (if (= (ekko/scene:rect-x pane) 0) 1 2)
                                                   1 9 0 fragment-id)
                   (floor (ekko/scene:rect-x rect) 8)
                   (floor (ekko/scene:rect-y rect) 16)
                   (checkerboard-payload src)
                   :width (ekko/scene:rect-width src)
                   :height (ekko/scene:rect-height src)
                   :cells-width (/ dx 8) :cells-height (/ dy 16))))
            (push transaction transactions))))))
    (setf transactions (nreverse transactions))
    (dolist (transaction transactions) (ekko/client:enqueue-transaction client transaction))
    transactions))

(defun make-native-checkerboard-transactions (client)
  "Queue exact-size RGB crops with Kitty's uppercase X/Y offsets." 
  (let ((source (ekko/scene:make-rect 0 0 26 18)) (transactions '()))
    (dolist (pane (list (ekko/scene:make-rect 0 0 24 16)
                        (ekko/scene:make-rect 32 0 24 16)))
      (let ((destination (ekko/scene:make-rect (- (ekko/scene:rect-x pane) 1) -1 26 18))
            (overlay (ekko/scene:make-rect (+ (ekko/scene:rect-x pane) 9) 3 6 9))
            (n 0))
        (dolist (fragment (ekko/scene:clip-placement destination source pane pane pane (list overlay)))
          (incf n)
          (let* ((rect (ekko/scene:mapped-fragment-destination fragment))
                 (src (ekko/scene:mapped-fragment-source fragment))
                 (x (floor (ekko/scene:rect-x rect) 8))
                 (y (floor (ekko/scene:rect-y rect) 16))
                 (xo (mod (ekko/scene:rect-x rect) 8))
                 (yo (mod (ekko/scene:rect-y rect) 16)))
            (push (kitty-transaction
                   (ekko/client:allocate-outer-id client (if (zerop (ekko/scene:rect-x pane)) 1 2) 1 10 0 n)
                   x y (native-payload (ekko/scene:rect-width src) (ekko/scene:rect-height src)
                                       (ekko/scene:rect-x src) (ekko/scene:rect-y src))
                   :width (ekko/scene:rect-width src) :height (ekko/scene:rect-height src)
                   :native-p t :x-offset xo :y-offset yo)
                 transactions)))))
    (setf transactions (nreverse transactions))
    (dolist (transaction transactions) (ekko/client:enqueue-transaction client transaction))
    transactions))

(defun replay (client &key (fixture :red-blue))
  (ecase fixture
    (:red-blue (make-demo-transactions client))
    (:checkerboard (make-checkerboard-transactions client))
    (:native-checkerboard (make-native-checkerboard-transactions client))))

(defun write-demo (pathname)
  (let ((client (ekko/client:make-attachment :max-bytes 4096 :max-transactions 8)))
    (unwind-protect
         (progn
           (let ((fixture (or (uiop:getenv "EKKO_GRAPHICS_FIXTURE") "red-blue")))
             (cond ((string= fixture "red-blue") (replay client :fixture :red-blue))
                   ((string= fixture "checkerboard") (replay client :fixture :checkerboard))
                   ((string= fixture "native") (replay client :fixture :native-checkerboard))
                   (t (error "Unknown EKKO_GRAPHICS_FIXTURE: ~A" fixture))))
                (with-open-file (stream pathname :direction :output :if-exists :supersede
                                        :element-type '(unsigned-byte 8))
                  (loop until (eq :drained
                                  (ekko/client:flush-transactions client
                                    (lambda (vector start count)
                                      (let ((amount (min 3 count)))
                                        (write-sequence vector stream :start start :end (+ start amount)) amount))
                                    :max-bytes most-positive-fixnum)))
                  (finish-output stream)))
      (ekko/client:attachment-teardown client))))

(defun executable-main ()
  (sb-ext:exit :code
   (let ((arguments (cdr sb-ext:*posix-argv*)))
    (if (= (length arguments) 1)
        (handler-case (progn (write-demo (first arguments)) 0)
          (error (condition) (format *error-output* "ekko graphics demo: ~A~%" condition) 2))
        (progn (format *error-output* "usage: ekko-graphics-demo OUTPUT-FILE~%") 2)))))
