(in-package #:ekko/runtime)

(defconstant +extension-packet-limit+ 65536)
(defstruct extension-worker process input output source path registry request deadline recovery)
(defun config-path ()
  (or (uiop:getenv "EKKO_CONFIG")
      (format nil "~A/ekko/init.lisp" (or (uiop:getenv "XDG_CONFIG_HOME")
                                        (namestring (merge-pathnames ".config" (user-homedir-pathname)))))))
(defun config-source (path)
  (if (probe-file path)
      (with-open-file (in path)
        (when (> (file-length in) 32768) (error "Configuration exceeds 32 KiB: ~A" path))
        (let ((source (make-string (file-length in))))
          (subseq source 0 (read-sequence source in))))
      (if (uiop:getenv "EKKO_CONFIG") (error "Configuration does not exist: ~A" path) "")))
(defun extension-data (bytes)
  (when (> (length bytes) +extension-packet-limit+) (error "Extension message too large"))
  (let ((data (decode-scene bytes)) (budget 8192))
    (labels ((check (value depth)
               (when (or (minusp (decf budget)) (> depth 40)) (error "Extension data too complex"))
               (typecase value
                 (cons (check (car value) (1+ depth)) (check (cdr value) depth))
                 (string (when (> (length value) 32768) (error "Extension string too long")))
                 (integer (unless (<= (- (expt 2 53)) value (expt 2 53)) (error "Extension integer too large")))
                 (symbol (unless (or (null value) (eq value t) (keywordp value)) (error "Extension symbol is not data")))
                 (t (error "Invalid extension value")))))
      (check data 0)) data))
(defun extension-send (wire value)
  (let ((bytes (encode-scene value)))
    (when (> (length bytes) +extension-packet-limit+) (error "Extension result exceeds 64 KiB"))
    (send-packet wire 30 bytes)))
(defun stop-worker (worker)
  (when worker
    (let ((process (extension-worker-process worker)))
      (when (sb-ext:process-alive-p process) (sb-ext:process-kill process 9))
      ;; The PTY reactor owns SIGCHLD and reaps by PID. SBCL's process-wait
      ;; depends on its own signal handler, which is intentionally disabled here.
      (loop while (= -11 (reap (sb-ext:process-pid process))) do (poll-fds nil 1))
      ;; Process streams own these descriptors; do not close them a second time.
      (sb-ext:process-close process))))
(defun start-worker (source path log)
  (let* ((process (sb-ext:run-program (car sb-ext:*posix-argv*) '("--extension-worker")
                                    :wait nil :input :stream :output :stream :error log :if-error-exists :append))
         (in (make-wire :fd (sb-sys:fd-stream-fd (sb-ext:process-output process)) :packet-limit (1+ +extension-packet-limit+)))
         (out (make-wire :fd (sb-sys:fd-stream-fd (sb-ext:process-input process))))
         (worker (make-extension-worker :process process :input in :output out :source source :path path
                                        :deadline (+ (now) 5) :request :load)) (ready nil))
    (unwind-protect
         (progn
           (checked (ekko/platform::nonblock (wire-fd in)))
           (checked (ekko/platform::nonblock (wire-fd out)))
           (extension-send out (list :load source path))
           (setf ready t) worker)
      (unless ready (stop-worker worker)))))
(defun worker-events (worker)
  (when worker
    (append (list (cons (wire-fd (extension-worker-input worker)) 1))
            (when (wire-queue (extension-worker-output worker))
              (list (cons (wire-fd (extension-worker-output worker)) 4))))))
(defun poll-worker (worker buffer)
  (unless (sb-ext:process-alive-p (extension-worker-process worker)) (error "Extension worker exited"))
  (when (and (extension-worker-request worker) (> (now) (extension-worker-deadline worker)))
    (error "Extension ~A timed out" (extension-worker-request worker)))
  (flush-wire (extension-worker-output worker))
  (let ((packets (receive-packets (extension-worker-input worker) buffer)))
    (when (> (length packets) 1) (error "Unexpected extension responses"))
    (when packets
      (unless (= (aref (first packets) 0) 30) (error "Invalid extension packet"))
      (extension-data (subseq (first packets) 1)))))
(defun load-worker (source path log)
  ;; Startup/check only: no running PTYs are held up by this wait. Reload uses
  ;; the normal reactor and swaps the candidate only after validation.
  (let ((worker (start-worker source path log)) (ready nil) (buffer (octets 65536)))
    (unwind-protect
         (loop for response = (poll-worker worker buffer) do
           (when response
             (when (eq (first response) :error) (error "~A: ~A" path (second response)))
             (unless (eq (first response) :ready) (error "Invalid extension startup"))
             (setf (extension-worker-registry worker) (second response)
                   (extension-worker-request worker) nil ready t)
             (return worker))
           (poll-fds (worker-events worker) 10))
      (unless ready (stop-worker worker)))))

(defun extension-worker-main ()
  (initialize)
  (checked (ekko/platform::nonblock 0)) (checked (ekko/platform::nonblock 1))
  (let ((input (make-wire :fd 0 :packet-limit (1+ +extension-packet-limit+))) (output (make-wire :fd 1)) (buffer (octets 65536)))
    (loop
      (dolist (packet (receive-packets input buffer))
        (let ((*standard-output* *error-output*) (*trace-output* *error-output*))
          (handler-case
              (let ((request (extension-data (subseq packet 1))))
                (case (first request)
                  (:load
                   (setf ekko/extensions::*components* nil)
                   (let ((package (find-package :ekko/builtins)))
                     (when package (funcall (find-symbol "INSTALL" package))))
                   (let* ((path (pathname (third request)))
                          (*default-pathname-defaults* (uiop:pathname-directory-pathname path)))
                     (load (make-string-input-stream (second request)) :verbose nil :print nil))
                   (extension-send output (list :ready (ekko/extensions::registry))))
                  (:dispatch
                   (extension-send output
                     (list :result (apply #'ekko/extensions::dispatch (rest request)))))
                  (otherwise (error "Unknown extension request"))))
            (error (e) (extension-send output (list :error (subseq (princ-to-string e) 0 (min 2000 (length (princ-to-string e))))))))))
      (flush-wire output)
      (poll-fds (append (list '(0 . 1)) (when (wire-queue output) (list '(1 . 4)))) 1000))))
