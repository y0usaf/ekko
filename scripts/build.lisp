(let ((sb-c::*source-namestring* "SYS:CONTRIB;ASDF.FASL")) (require "asdf"))
(let* ((root (truename (or (uiop:getenv "EKKO_SOURCE_DIR") ".")))
       (asdf:*central-registry* nil))
  (push root asdf:*central-registry*)
  (asdf:load-system (or (uiop:getenv "EKKO_BUILD_SYSTEM") "ekko"))
  (asdf:clear-configuration)
  (mapc #'asdf:clear-system (asdf:registered-systems))
  (sb-ext:save-lisp-and-die
   (or (uiop:getenv "EKKO_OUTPUT") "ekko")
   :toplevel (symbol-function (find-symbol "EXECUTABLE-MAIN" "EKKO"))
   :executable t
   :save-runtime-options t))
