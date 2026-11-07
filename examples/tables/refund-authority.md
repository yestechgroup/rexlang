> # Refund approval authority
> Who must approve a refund and how much each authority level may release.

| U | requested_cents | requires_approval | approver_role | maximum_amount |
|:-:|:---------------:|:-----------------:|:-------------:|:--------------:|
|   |       `i`       |        `o`        |      `o`      |      `o`       |
| 1 |     <=5000      |       false       |  "agent"      |      5000      |
| 2 | [5001..100000]  |       true        |  "finance"    |     100000     |
| 3 |     >100000     |       true        |   "board"     |    1000000     |
