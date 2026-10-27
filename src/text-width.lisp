(defpackage #:ekko/text (:use #:cl)
  (:export #:display-width #:code-info #:cluster-step #:cluster-state)
  (:documentation "UNICODE LICENSE V3

COPYRIGHT AND PERMISSION NOTICE

Copyright © 1991-2026 Unicode, Inc.

NOTICE TO USER: Carefully read the following legal agreement. BY
DOWNLOADING, INSTALLING, COPYING OR OTHERWISE USING DATA FILES, AND/OR
SOFTWARE, YOU UNEQUIVOCALLY ACCEPT, AND AGREE TO BE BOUND BY, ALL OF THE
TERMS AND CONDITIONS OF THIS AGREEMENT. IF YOU DO NOT AGREE, DO NOT
DOWNLOAD, INSTALL, COPY, DISTRIBUTE OR USE THE DATA FILES OR SOFTWARE.

Permission is hereby granted, free of charge, to any person obtaining a
copy of data files and any associated documentation (the \"Data Files\") or
software and any associated documentation (the \"Software\") to deal in the
Data Files or Software without restriction, including without limitation
the rights to use, copy, modify, merge, publish, distribute, and/or sell
copies of the Data Files or Software, and to permit persons to whom the
Data Files or Software are furnished to do so, provided that either (a)
this copyright and permission notice appear with all copies of the Data
Files or Software, or (b) this copyright and permission notice appear in
associated Documentation.

THE DATA FILES AND SOFTWARE ARE PROVIDED \"AS IS\", WITHOUT WARRANTY OF ANY
KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT OF
THIRD PARTY RIGHTS.

IN NO EVENT SHALL THE COPYRIGHT HOLDER OR HOLDERS INCLUDED IN THIS NOTICE
BE LIABLE FOR ANY CLAIM, OR ANY SPECIAL INDIRECT OR CONSEQUENTIAL DAMAGES,
OR ANY DAMAGES WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS,
WHETHER IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION,
ARISING OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THE DATA
FILES OR SOFTWARE.

Except as contained in this notice, the name of a copyright holder shall
not be used in advertising or otherwise to promote the sale, use or other
dealings in these Data Files or Software without prior written
authorization of the copyright holder.
"))
(in-package #:ekko/text)
(defparameter *code-index*
  (let ((table (make-array 8704 :element-type '(unsigned-byte 8)))
        (start 0))
    (loop for (count value) on '(
      1 0 1 1 4 2 1 3 2 2 1 4 1 2 1 5 1 6 1 7 1 8 1 9
      1 10 1 11 1 12 1 13 1 14 1 15 1 16 1 17 1 18 1 19 1 20 1 21
      1 22 1 23 1 24 1 25 1 26 1 27 1 28 3 2 1 29 7 2 1 30 1 31
      1 32 1 33 1 34 1 2 1 35 1 36 1 37 1 38 1 39 1 40 1 2 1 41
      4 2 1 42 1 43 1 44 1 45 2 2 1 46 1 47 1 2 1 48 1 2 1 49
      1 50 1 51 1 52 1 53 2 2 1 54 3 2 1 55 2 2 1 56 1 57 1 58
      1 2 1 59 1 60 1 61 1 62 1 63 1 64 1 65 1 66 1 67 227 60 1 68
      2 2 1 69 1 70 2 2 1 71 1 72 1 73 1 74 1 75 1 76 1 2 1 77
      87 60 1 78 16 2 1 79 1 80 1 81 1 82 1 83 1 84 1 85 1 86 1 87
      1 2 1 88 2 2 1 89 5 2 1 90 1 2 1 91 1 92 1 93 25 2 1 94
      2 60 1 95 1 96 1 97 5 2 1 98 1 99 1 100 1 101 3 2 1 102 1 2
      1 103 1 104 13 2 1 105 1 106 4 2 1 107 2 2 1 108 1 109 1 110 1 111
      1 112 1 113 1 114 1 115 1 116 1 117 1 118 1 119 1 120 1 2 1 121 1 122
      1 123 1 124 1 2 1 125 1 2 1 126 1 127 1 128 1 129 1 130 1 2 1 131
      1 132 1 133 1 134 1 2 1 135 1 136 41 2 1 137 89 2 1 138 18 2 1 139
      1 140 7 2 1 141 1 142 47 60 1 143 9 60 1 144 1 145 68 2 1 146 2 60
      1 147 2 60 1 148 19 2 1 149 36 2 1 150 3 2 1 151 1 152 1 153 1 2
      1 154 13 2 1 155 1 156 10 2 1 157 1 158 1 140 2 2 1 159 3 2 1 160
      1 2 1 161 1 2 1 162 3 2 1 163 1 164 13 2 1 165 1 166 1 167 1 168
      1 169 1 170 1 171 1 172 1 173 1 174 1 175 1 176 1 177 1 178 1 2 1 179
      1 180 1 181 1 182 1 183 1 184 1 185 2 2 7 170 1 186 333 60 1 187 32 60
      1 188 1 60 1 189 44 60 1 190 57 60 1 191 4 60 1 192 19 2 4 60 1 193
      11 2 38 60 1 194 32 60 1 195 5560 2 1 196 1 197 1 198 1 199 28 197 525 2
      1 200 5 2 1 201 29 2 1 202 942 2
    ) by #'cddr
          do (fill table value :start start :end (+ start count))
             (incf start count))
    table))
(declaim (type (simple-array (unsigned-byte 8) (8704)) *code-index*))
(defparameter *code-blocks*
  (let ((table (make-array 25984 :element-type '(unsigned-byte 8)))
        (start 0))
    (loop for (count value) on '(
      35 1 1 129 6 1 1 129 5 1 10 129 111 1 1 137 4 1 1 137 209 1 79 4
      1 36 32 4 19 1 7 4 135 1 45 4 1 1 1 4 1 1 2 4 1 1 2 4
      1 1 1 4 56 1 6 0 10 1 11 4 1 1 1 32 46 1 21 4 16 1 1 4
      101 1 7 4 1 0 1 1 6 4 2 1 2 4 1 1 4 4 33 1 1 0 1 1
      1 4 30 1 27 4 91 1 11 4 58 1 9 4 9 1 1 4 24 1 4 4 1 1
      9 4 1 1 3 4 1 1 5 4 43 1 3 4 52 1 2 0 5 1 9 4 42 1
      24 4 1 0 32 4 1 0 54 1 1 4 1 0 1 4 1 1 3 0 8 4 4 0
      1 4 2 0 1 1 7 4 10 1 2 4 29 1 1 4 2 0 56 1 1 4 1 1
      1 68 2 0 4 4 2 1 2 0 2 1 2 0 1 4 9 1 1 68 10 1 2 4
      26 1 1 4 2 1 2 4 1 0 56 1 1 4 1 1 3 0 2 4 4 1 2 4
      2 1 3 4 3 1 1 4 30 1 2 4 3 1 1 4 11 1 2 4 1 0 56 1
      1 4 1 1 3 0 5 4 1 1 2 4 1 0 1 1 2 0 1 4 20 1 2 4
      22 1 6 4 1 1 1 4 2 0 56 1 1 4 1 1 1 68 1 4 1 0 4 4
      2 1 2 0 2 1 2 0 1 4 7 1 2 4 1 68 10 1 2 4 30 1 1 4
      59 1 1 68 1 0 1 4 2 0 3 1 3 0 1 1 3 0 1 4 9 1 1 68
      40 1 1 4 3 0 1 4 55 1 1 4 1 1 3 4 4 0 1 1 3 4 1 1
      4 4 7 1 2 4 11 1 2 4 29 1 1 4 2 0 56 1 1 4 1 1 1 0
      1 4 1 68 1 0 1 68 2 0 1 1 1 4 2 68 1 1 2 68 2 4 7 1
      2 68 11 1 2 4 28 1 2 4 2 0 55 1 2 4 1 1 1 68 2 0 4 4
      1 1 3 0 1 1 3 0 1 4 9 1 1 68 10 1 2 4 29 1 1 4 2 0
      70 1 1 4 4 1 1 68 2 0 3 4 1 1 1 4 1 1 7 0 1 68 18 1
      2 0 61 1 1 4 2 1 7 4 12 1 8 4 98 1 1 4 2 1 9 4 11 1
      7 4 73 1 2 4 27 1 1 4 1 1 1 4 1 1 1 4 4 1 2 0 49 1
      14 4 1 0 5 4 1 1 2 4 5 1 11 4 1 1 36 4 9 1 1 4 100 1
      2 0 4 4 1 0 6 4 1 0 2 4 2 0 2 4 23 1 2 0 2 4 4 1
      3 4 1 1 3 0 2 1 7 0 3 1 4 4 13 1 1 4 2 0 2 4 6 0
      1 4 1 1 1 0 10 1 3 0 1 4 98 1 95 2 1 34 1 33 124 1 3 4
      50 1 3 4 1 68 28 1 2 4 1 68 29 1 2 4 30 1 2 4 64 1 2 36
      1 0 7 4 8 0 1 4 2 0 11 4 9 1 1 4 45 1 3 36 1 32 1 36
      117 1 2 4 34 1 1 4 118 1 3 4 4 0 2 4 3 0 4 1 2 0 1 4
      6 0 3 4 91 1 2 4 2 0 1 4 57 1 1 0 1 4 1 0 7 4 1 1
      1 4 1 0 1 4 2 0 8 4 6 0 10 4 2 1 1 4 48 1 45 4 1 5
      2 1 12 4 20 1 4 4 1 0 47 1 1 4 1 68 5 4 1 68 1 4 1 68
      4 0 1 4 2 68 38 1 9 4 12 1 2 4 1 0 30 1 1 0 4 4 2 0
      2 4 1 68 3 4 56 1 1 4 1 0 2 4 3 0 1 4 1 0 3 4 2 68
      48 1 8 0 8 4 2 0 2 4 152 1 3 4 1 1 13 4 1 0 7 4 4 1
      1 4 6 1 1 4 2 1 1 0 2 4 70 1 64 4 11 1 1 32 1 36 3 32
      26 1 5 32 13 1 1 137 12 1 1 137 22 1 5 32 1 33 10 32 96 1 33 4
      49 1 1 137 22 1 1 137 90 1 6 137 15 1 2 137 111 1 2 138 12 1 1 137
      2 2 164 1 1 137 25 1 4 138 3 137 1 138 2 137 1 138 4 1 3 137 71 1
      1 137 103 1 2 137 10 1 1 137 9 1 1 137 58 1 2 137 2 138 1 1 5 137
      9 1 1 137 2 1 1 137 2 1 2 138 2 1 1 137 4 1 1 153 2 1 1 137
      1 1 2 137 2 1 1 137 3 1 1 137 3 1 2 137 8 2 3 137 5 1 1 137
      1 1 1 137 5 1 12 138 11 1 2 137 2 1 1 137 1 1 2 137 1 1 1 137
      18 1 1 137 2 1 1 137 1 138 10 1 6 2 2 1 1 137 1 138 4 137 1 1
      1 137 1 1 2 137 3 1 1 137 1 138 5 1 1 137 2 1 2 138 4 1 2 137
      11 1 2 138 5 1 2 138 2 1 1 137 5 1 1 138 1 137 1 1 1 137 1 1
      1 137 1 138 20 1 1 137 1 138 5 1 2 137 2 138 1 137 1 138 1 1 2 137
      1 153 1 138 2 1 1 138 4 1 1 137 2 1 1 138 2 1 2 137 2 154 2 153
      1 1 1 137 2 1 1 137 1 1 1 137 1 1 1 137 6 1 1 137 3 1 1 137
      6 1 1 138 10 1 2 137 15 1 1 137 2 1 1 137 4 1 1 138 1 1 1 138
      4 1 3 138 1 1 1 138 11 1 2 137 48 1 3 138 9 1 1 137 14 1 1 138
      14 1 1 138 116 1 2 137 79 1 3 137 19 1 2 138 51 1 1 138 4 1 1 138
      153 1 3 4 141 1 1 4 96 1 32 4 26 2 1 1 89 2 12 1 214 2 26 1
      58 2 4 4 2 68 1 138 12 2 1 138 1 2 2 1 86 2 2 1 2 4 101 2
      5 1 43 2 1 1 51 2 1 34 42 2 1 1 86 2 9 1 48 2 1 1 40 2
      8 1 71 2 1 138 1 2 1 138 115 2 3 1 55 2 168 1 4 4 1 1 10 4
      32 1 2 4 80 1 2 4 16 1 1 4 3 1 1 4 4 1 1 4 23 1 2 0
      2 4 1 0 4 1 1 4 83 1 2 0 50 1 16 0 2 4 26 1 18 4 13 1
      1 4 38 1 8 4 25 1 11 4 1 0 1 68 12 1 29 2 3 1 3 4 1 0
      47 1 1 4 2 0 4 4 2 0 2 4 2 0 1 68 36 1 1 4 67 1 6 4
      2 0 2 4 2 0 2 4 12 1 1 4 8 1 1 4 1 0 45 1 1 0 1 4
      1 0 50 1 1 4 1 1 3 4 2 1 2 4 5 1 2 4 1 1 1 4 41 1
      1 0 2 4 2 0 5 1 1 0 1 4 108 1 2 0 1 4 2 0 1 4 2 0
      1 1 1 0 1 4 18 1 36 2 156 1 24 0 1 1 2 0 34 1 1 0 108 1
      3 0 65 1 1 0 1 1 9 0 76 1 8 0 93 1 1 0 107 1 4 0 101 1
      1 0 80 1 4 0 208 1 5 0 44 1 5 0 1 1 4 0 163 1 3 0 2 1
      2 0 139 1 18 0 31 1 11 0 15 1 65 0 140 1 3 0 49 1 11 0 134 1
      9 0 7 1 9 0 7 1 9 0 101 1 1 0 5 1 2 0 1 1 1 0 55 1
      110 2 2 1 106 2 68 1 1 4 97 1 16 36 10 2 6 1 16 4 35 2 1 1
      19 2 1 1 4 2 147 1 1 32 1 1 96 2 61 1 2 69 1 33 63 1 7 2
      9 1 9 33 3 0 129 1 1 4 98 1 1 4 149 1 5 4 6 1 3 4 1 1
      2 4 5 1 4 4 40 1 3 4 4 1 1 4 165 1 2 4 61 1 4 4 65 1
      5 4 61 1 2 4 77 1 6 4 70 1 11 4 49 1 4 4 122 1 1 0 1 4
      1 0 53 1 15 4 41 1 1 4 2 1 2 4 10 1 3 4 1 0 45 1 3 0
      4 4 2 0 2 4 2 1 1 0 4 1 1 4 10 1 1 0 50 1 3 4 36 1
      5 4 1 0 8 4 16 1 2 0 44 1 1 4 12 1 2 4 1 0 48 1 3 0
      9 4 1 0 1 68 8 1 4 4 1 1 1 0 1 4 92 1 3 0 3 4 2 0
      1 4 1 68 2 4 6 1 1 4 2 1 1 4 157 1 1 4 3 0 8 4 21 1
      2 4 2 0 55 1 2 4 1 1 1 68 1 0 1 4 4 0 2 1 2 0 2 1
      2 0 1 68 9 1 1 68 10 1 2 0 2 1 7 4 3 1 5 4 67 1 1 68
      2 0 6 4 1 1 1 68 2 1 1 68 1 1 3 68 1 0 1 1 2 0 1 4
      1 68 1 4 1 1 1 4 14 1 2 4 82 1 3 0 8 4 2 0 3 4 1 0
      1 4 23 1 1 4 81 1 1 68 2 0 6 4 1 0 1 4 2 0 1 68 1 0
      2 4 1 0 2 4 107 1 1 68 2 0 4 4 2 1 4 0 2 4 1 0 2 4
      27 1 2 4 82 1 3 0 8 4 2 0 1 4 1 0 2 4 106 1 1 4 1 0
      1 4 2 0 6 4 1 68 1 4 101 1 1 4 1 0 1 4 2 0 4 4 1 0
      5 4 128 1 3 0 9 4 1 0 2 4 117 1 1 68 5 0 1 1 2 0 2 1
      2 4 1 68 1 4 4 0 1 4 141 1 3 0 4 4 2 1 2 4 4 0 1 4
      3 1 1 0 28 1 10 4 40 1 6 4 2 0 4 4 8 1 1 4 9 1 6 4
      2 0 3 4 42 1 4 0 13 4 1 0 2 4 198 1 1 4 1 0 3 4 1 0
      1 4 1 0 71 1 1 0 7 4 1 1 6 4 1 0 1 4 82 1 22 4 1 1
      1 0 7 4 1 0 2 4 1 0 2 4 122 1 6 4 3 1 1 4 1 1 2 4
      1 1 7 4 1 0 1 4 66 1 5 0 1 1 2 4 1 1 2 0 1 4 1 0
      1 4 219 1 2 4 2 0 9 1 2 4 1 1 1 0 48 1 2 0 5 4 3 1
      2 0 1 4 1 68 1 4 23 1 1 4 85 1 16 0 1 4 6 1 15 4 72 1
      12 4 3 0 3 4 192 1 5 4 59 1 7 4 152 1 1 4 1 1 55 0 7 1
      4 4 77 1 4 2 1 4 11 1 2 68 14 1 120 2 8 1 86 2 41 1 10 2
      231 1 4 2 1 1 7 2 1 1 2 2 1 1 35 2 15 1 1 2 29 1 3 2
      2 1 1 2 14 1 4 2 8 1 140 2 33 1 2 4 1 1 4 32 92 1 46 4
      2 1 23 4 158 1 2 68 3 4 3 1 6 68 8 32 8 4 2 1 7 4 20 1
      1 0 9 1 4 4 148 1 3 4 59 1 87 2 9 1 23 2 9 1 55 4 4 1
      50 4 8 1 1 4 14 1 1 4 22 1 5 4 1 1 15 4 80 1 7 4 1 1
      17 4 2 1 7 4 1 1 2 4 1 1 5 4 100 1 1 4 158 1 1 4 61 1
      4 4 124 1 4 4 126 1 2 4 115 1 1 4 2 1 1 4 7 1 2 4 5 1
      1 4 90 1 7 4 109 1 7 4 57 1 1 138 39 1 4 9 100 1 12 9 15 1
      2 9 15 1 1 9 14 1 1 10 1 9 37 1 10 9 112 1 2 137 12 1 2 137
      14 1 1 10 2 1 10 10 19 1 56 9 26 1 1 2 1 10 1 138 13 9 10 2
      1 138 20 2 1 138 2 2 5 10 1 138 3 10 1 2 4 9 9 2 7 9 2 10
      14 9 6 2 154 9 13 10 3 138 5 10 1 138 6 10 1 138 4 10 1 137 2 1
      9 137 9 10 1 137 65 10 1 138 4 10 1 137 7 10 1 26 13 10 1 138 2 1
      2 137 1 1 3 137 2 1 2 137 7 10 1 138 4 10 3 138 19 10 1 154 1 26
      1 154 1 10 1 138 1 26 2 10 1 154 2 153 2 137 5 10 12 137 1 138 12 10
      1 138 3 10 2 1 1 137 1 10 1 137 1 1 1 137 3 10 5 2 8 10 1 138
      12 10 1 138 9 10 1 138 6 10 1 138 24 10 1 137 1 10 1 137 1 154 1 26
      2 10 4 154 3 26 2 154 2 26 2 10 1 138 18 10 4 26 1 154 14 26 3 10
      1 26 1 138 3 10 3 26 1 10 3 26 7 10 1 26 1 10 1 26 17 10 1 138
      6 10 1 26 5 10 1 138 2 10 1 138 7 10 1 138 3 10 1 138 11 10 1 138
      14 10 1 138 4 10 1 138 4 10 3 138 3 10 4 138 9 10 1 138 1 10 3 138
      1 10 1 137 1 1 9 10 1 138 4 10 1 138 4 10 2 138 42 10 11 1 2 137
      4 10 1 1 24 138 7 1 2 137 2 1 1 137 2 153 4 137 1 26 12 1 1 137
      2 1 4 137 2 1 1 153 4 1 2 26 13 1 1 10 1 137 2 1 1 137 8 1
      2 137 9 1 1 137 5 1 3 137 12 1 3 137 8 1 3 137 2 1 1 137 1 1
      1 137 4 1 1 137 6 1 1 137 3 1 1 137 6 1 1 137 21 10 1 138 52 10
      3 26 3 10 5 26 48 1 7 10 1 138 5 10 1 138 3 10 1 138 2 10 1 138
      3 10 1 138 10 10 1 26 9 10 1 138 4 10 1 138 1 10 3 26 2 10 2 138
      1 10 1 138 3 10 1 26 5 10 5 1 1 137 1 26 3 137 3 10 2 1 3 10
      4 9 4 10 6 137 3 1 1 137 1 1 2 10 3 9 1 137 2 1 1 137 9 10
      3 9 90 1 6 9 12 10 4 9 1 10 15 9 12 1 4 9 56 1 8 9 10 1
      6 9 40 1 8 9 30 1 2 9 12 1 4 9 2 1 14 9 9 1 39 9 12 1
      1 26 2 10 1 26 8 10 8 26 6 10 1 26 9 10 10 26 1 10 1 1 3 26
      7 10 1 1 48 10 1 26 61 10 2 26 1 10 2 26 1 10 1 26 17 10 3 26
      1 10 13 26 34 10 88 1 8 9 14 1 2 9 13 10 3 9 10 10 5 9 52 10
      3 26 1 10 7 9 15 10 2 9 11 10 6 9 9 26 133 9 2 1 96 2 32 1
      58 2 6 1 94 2 2 1 130 2 14 1 177 2 15 1 110 2 34 1 30 2 98 1
      75 2 5 1 96 2 80 1 1 33 1 32 30 33 96 36 128 33 240 36 16 33 74 1
      5 0 67 1 10 0 144 1 4 0 8 1 1 0 2 1 1 0 4 1 10 0 54 1
    ) by #'cddr
          do (fill table value :start start :end (+ start count))
             (incf start count))
    table))
(declaim (type (simple-array (unsigned-byte 8) (25984)) *code-blocks*))
(declaim (inline code-info))
(defun code-info (codepoint)
  "Return cudaterm's scalar width and cluster properties for a Unicode codepoint."
  (declare (type (integer 0 #x10ffff) codepoint))
  (aref *code-blocks*
        (+ (ash (aref *code-index* (ash codepoint -7)) 7)
           (logand codepoint 127))))
(defun cluster-step (state codepoint)
  "Return next cluster state, added cells, and whether CODEPOINT attaches to STATE."
  (declare (type (unsigned-byte 7) state)
           (type (integer 0 #x10ffff) codepoint))
  (let* ((info (code-info codepoint))
         (old-width (logand state 3))
         (width (logand info 3))
         (modifier (<= #x1f3fb codepoint #x1f3ff))
         (extend (logtest info 4))
         (ignorable (logtest info 32))
         (attach-modifier (and (plusp old-width) modifier (logtest state 8)))
         (attach-extend (and (plusp old-width) extend))
         (attach-zwj (and (plusp old-width) (logtest info 8) (logtest state 32))))
    (if (or (zerop width) ignorable attach-modifier attach-extend attach-zwj)
        (let* ((promote (and (= old-width 1)
                             (or attach-modifier attach-zwj
                                 (and attach-extend (logtest info 64))
                                 (and (= codepoint #xfe0f)
                                      (not (logtest state 4)) (logtest state 64)))))
               (next (logior (if promote 2 old-width) 4 (logand state 64))))
          (when (if (= codepoint #xfe0f)
                    (logtest state 8)
                    (and (not ignorable) (not extend) (not modifier)
                         (logtest info 16)))
            (setf next (logior next 8)))
          (cond
            ((= codepoint #x200d)
             (when (logtest state 16) (setf next (logior next 32))))
            (ignorable (setf next (logior next (logand state 48))))
            ((or extend modifier)
             (setf next (logior next (logand state 16))))
            ((logtest info 8) (setf next (logior next 16))))
          (values next (if promote 1 0) (plusp old-width)))
        (values (logior width
                        (if (logtest info 16) 8 0)
                        (if (logtest info 8) 16 0)
                        (if (logtest info 128) 64 0))
                width nil))))
(defun cluster-state (text)
  "Return the cluster state for one existing grid cell's text."
  (let ((state 0))
    (loop for character across text
          do (setf state (cluster-step state (char-code character))))
    state))
(defun display-width (value)
  "Return cudaterm's scalar character width or incremental string cell width."
  (etypecase value
    (character (logand (code-info (char-code value)) 3))
    (string
     (let ((state 0) (width 0))
       (loop for character across value
             do (multiple-value-bind (next added) (cluster-step state (char-code character))
                  (setf state next)
                  (incf width added)))
       width))))
