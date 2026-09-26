;; Windows XP (Luna blue) for the desktop. Load it after any other profile, or
;; append it to init.lisp. Blue line frames and solid title bars, a blue
;; taskbar with a green start button, white menus with blue selection, yellow
;; balloon toasts, and a Bliss-like sky and hill as the ground, seen dimmed
;; through every window.
(in-package #:cl-user)

(setf ekko/desktop:*theme*
      (list* :accents '((0 84 227))
             :titlebar :filled :title-align :left
             :frame-inactive '(122 150 223) :title-inactive '(122 150 223)
             :title-text '(255 255 255) :title-inactive-text '(216 228 248)
             :control-bg '(38 110 245) :close-bg '(218 70 32)
             :bar-bg '(36 94 220) :bar-text '(255 255 255)
             :start '(" ❖ start " (255 255 255) (60 150 50))
             :tray-bg '(17 144 232) :tray-text '(255 255 255)
             :entry-bg '(60 129 243) :entry-text '(255 255 255)
             :entry-active '(30 72 170) :entry-active-text '(255 255 255) :entry-hover '(90 150 250)
             :entry-minimized '(60 129 243) :entry-minimized-text '(210 225 250)
             :panel-bg '(255 255 255) :panel-text '(0 0 0) :panel-muted '(110 110 110)
             :panel-border '(122 150 223) :highlight '(49 106 197) :highlight-text '(255 255 255)
             :toast-bg '(255 255 225) :toast-text '(0 0 0) :toast-frame '(0 0 0)
             :corners '("┌" "┐" "└" "┘") :attention-style :mark :shadow '(20 40 90)
             ekko/desktop:*theme*))

;; The ground: sky and hill bands, darkened inside windows like tinted glass.
(ekko/extensions:register-component :id ':xp :reads '(:session)
                                    :handler (lambda (snapshot event) (declare (ignore snapshot event)) nil))
(ekko/extensions:set-option :component ':xp :name :ground
                            :value '((5 (58 110 196)) (4 (86 139 222)) (3 (125 170 235))
                                     (2 (111 164 64)) (3 (74 128 42))))
(ekko/extensions:set-option :component ':xp :name :ground-dim :value 65)
