(in-package #:ekko)

(defparameter *version* "0.1.0")

(defun version () *version*)

(defun usage (&optional (stream *standard-output*))
  (format stream "Ekko ~A~%~%Usage: ekko [OPTION]~%       ekko run [--session NAME] COMMAND [ARGS...] ::: COMMAND [ARGS...]~%       ekko attach [NAME]~%       ekko status [NAME]~%       ekko stop [NAME]~%       ekko doctor --restore-terminal~%~%Keys: Ctrl-b then Tab/1/2 focus, z zoom, s swap, </> divider, d detach, q quit.~%Default session: default. Sessions and apps survive closing the client.~%~%Options:~%  -h, --help     Show this help~%  --version      Show the version~%" *version*))

(defun runtime-command (arguments)
  (handler-case
      (let ((command (first arguments)) (args (rest arguments)))
        (cond
          ((member command '("run" "--serve") :test #'string=)
           (let ((name "default"))
             (when (string= command "--serve") (setf name (pop args)))
             (when (equal (first args) "--session") (pop args) (setf name (pop args)))
             (unless name (error "Missing session name"))
             (let* ((split (position ":::" args :test #'string=))
                    (commands (if split (list (subseq args 0 split) (subseq args (1+ split))) (list args))))
               (unless (every #'identity commands) (error "Each pane needs an executable"))
               (when (find ":::" (second commands) :test #'string=) (error "At most two panes are supported"))
               (if (string= command "--serve") (ekko/runtime:serve name commands) (ekko/runtime:run-session name commands)))))
          ((member command '("attach" "status" "stop") :test #'string=)
           (when (> (length args) 1) (error "Unexpected argument"))
           (if (string= command "attach") (ekko/runtime:attach-session (or (first args) "default"))
               (ekko/runtime:control-session (or (first args) "default") command)))
          ((and (string= command "doctor") (equal args '("--restore-terminal"))) (ekko/runtime:restore-terminal))
          (t (error "Unknown command ~A" command))))
    (error (condition) (format *error-output* "ekko: ~A~%" condition) 2)))

(defun main (&optional (arguments (cdr sb-ext:*posix-argv*)))
  (cond
    ((null arguments) (usage))
    ((or (string= (first arguments) "-h")
         (string= (first arguments) "--help"))
     (if (rest arguments)
         (progn (format *error-output* "ekko: unexpected argument ~A~%" (second arguments)) 2)
         (usage)))
    ((string= (first arguments) "--version")
     (if (rest arguments)
         (progn (format *error-output* "ekko: unexpected argument ~A~%" (second arguments)) 2)
         (format t "~A~%" *version*)))
    ((member (first arguments) '("run" "--serve" "attach" "status" "stop" "doctor") :test #'string=)
     (runtime-command arguments))
    (t
     (format *error-output* "ekko: unknown option ~A~%" (first arguments))
     (usage *error-output*)
     2)))

(defun executable-main ()
  (sb-ext:exit :code (or (main) 0)))
