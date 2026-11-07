> # Pay-run contributions
> The employee deduction, the employer contribution, the ESCT on it, and the
> compliance status the pay run records.

| U | gross_salary | employee_rate | savings_suspension | employee_deduction_required   | employer_cec_amount  | esct_amount | compliance_status | Legal reference |
|:-:|:------------:|:-------------:|:------------------:|:-----------------------------:|:--------------------:|:-----------:|:-----------------:|:---------------:|
|   |     `i`      |      `i`      |        `i`         |              `o`              |         `o`          |     `o`     |        `o`        |      `ann`      |
| 1 |      -       |       0       |         -          |             "No"              |          0           |      0      |    "Voluntary"    |   KS Act s 5A   |
| 2 |      -       |    [1..8]     |       false        |             "Yes"             |         1080         |     113     |    "Compliant"    |   KS Act s 5B   |
| 3 |      -       |    [1..8]     |        true        |             "No"              |         1080         |      0      |  "NonCompliant"   | KS Act s 5B(1A) |
