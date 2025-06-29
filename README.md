# hi! i'm munibot!

the friendliest, cutest, most lovable bot for Discord and Twitch, personality
included!

# runtime setup

## environment

i need some environment variables at runtime to run smoothly.

```env
# surrealdb database gateway
DATABASE_URL=127.0.0.1:7654

# surrealdb database endpoint
DATABASE_USER=munibot

# surrealdb database password
DATABASE_PASS=

# discord client details can be retrieved at <https://discord.com/developers/applications>
DISCORD_APPLICATION_ID=
DISCORD_CLIENT_SECRET=
DISCORD_PUBLIC_KEY=
DISCORD_TOKEN=

# twitch stuff can be retrieved from <https://dev.twitch.tv/console/apps>
TWITCH_CLIENT_ID=
TWITCH_CLIENT_SECRET=
TWITCH_TOKEN=
```

## surrealdb

here are some SurrealQL commands to run on your SurrealDB instance to set things
up.

### configuring an admin user

if your surrealdb instance doesn't have a root user yet, ensure the instance is
started with the flags `-u root -p root`, then login with
`surreal sql -e ws://127.0.0.1:7654 -u root -p root`.

if you want, you can create a root user:

```sql
DEFINE USER muni ON ROOT PASSWORD "m1lksh@kE!" ROLES OWNER;
```

**make sure you restart your instance without the `-u` and `-p` flags!**

### creating a `munibot` user

```sql
USE NS munibot;
USE DB munibot;
DEFINE USER munibot ON DATABASE PASSWORD "m1lksh@kE!" ROLES EDITOR;
```

add your password to the `.env` file:

```conf
DATABASE_PASS=m1lksh@kE!
```

# contact my creator

the best way to contact my creator is `@municorn` on Discord.

you may also contact him on Matrix: `@municorn:matrix.org`.

# copyright and license

The GPLv3 License (GPLv3)

munibot, municorn's Discord and Twitch bot\
Copyright (c) 2023-2025 municorn

this program is free software: you can redistribute it and/or modify it under
the terms of the GNU General Public License as published by the Free Software
Foundation, either version 3 of the License, or (at your option) any later
version.

this program is distributed in the hope that it will be useful, but WITHOUT ANY
WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A
PARTICULAR PURPOSE. see the GNU General Public License for more details.

a copy of the GNU General Public License is included along with this source code
as the LICENSE file. if it's lost for some reason, see
<http://www.gnu.org/licenses/>.
