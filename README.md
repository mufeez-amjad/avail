# avail
avail is a CLI tool that helps you find available times between all your calendars.

You can search for availabities based on:
- duration (e.g. one hour)
- within a search window (e.g. from start to end OR 1 week out)
- during specific hours of the day (e.g. 12pm onwards)

avail also lets you create hold events so you don't double book yourself.

![demo.gif](./assets/demo.gif)

## Installation
To install avail, run the following command:

```bash
curl -fsSL https://raw.githubusercontent.com/mufeez-amjad/avail/install.sh | sh -
```

You'll then need to retrieve OAuth credentials for Microsoft Outlook, Google Calendar, or both. Follow the instructions [here]().

## Usage
```
Usage: avail [OPTIONS] [COMMAND]

Commands:
  accounts   Manages OAuth accounts (Microsoft Outlook and Google Calendar)
  calendars  Allows specifying which calendars to use when querying, refreshes calendar cache for added accounts
  help       Print this message or the help of the given subcommand(s)

Options:
      --start <START>        Start of search window in the form of MM/DD/YYYY (default now)
      --end <END>            End of search window in the form of MM/DD/YYYY (default start + 7 days)
      --min <MIN>            Minimum time for availability in the form of <int>:<int>am/pm (default 9:00am)
      --max <MAX>            Maximum time for availability in the form of <int>:<int>am/pm (default 5:00pm)
  -w, --window <WINDOW>      Duration of search window, specify with <int>(w|d|h|m) (default 1w)
      --include-weekends     Option to include weekends in availability search (default false)
  -d, --duration <DURATION>  Duration of availability window, specify with <int>(w|d|h|m) (default 30m)
  -c, --create-hold-event    Create a hold event (default false)
  -h, --help                 Print help information
  -V, --version              Print version information
```

## Examples
Find 30 minute blocks of availability between 9:00am and 5:00pm from now until one week from now:

```
avail
```

Find 1 hour blocks of availability between 10:00am and 4:00pm from 01/01/2022 until 01/31/2022:

```bash
avail --start 01/01/2022 --end 01/31/2022 --min 10:00am --max 4:00pm --duration 1h
```

Find 2 hour blocks of availability between 9:00am and 5:00pm including weekends from now until 2 weeks from now:

```bash
avail --window 2w --include-weekends --duration 2h
```

## Contributing
We welcome contributions to avail! Please follow our contributing guidelines before opening a pull request.

## License
avail is licensed under the [MIT License](./LICENSE.md).
