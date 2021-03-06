<!-- This Source Code Form is subject to the terms of the Mozilla Public
   - License, v. 2.0. If a copy of the MPL was not distributed with this
   - file, You can obtain one at http://mozilla.org/MPL/2.0/. -->

Log requirements
==========

Premise: see "requirements.md" file.



Partitioning
------------

For a small enough set, a single log file will be sufficient. At some point, partitioning
the log becomes essential. For the reasons outlined in the *Requirements* document,
both partitioning history and partitioning elements should be allowed.

Additionally, it should be possible to find all elements in some user-defined subset (e.g.
a folder or tag) quickly. Partitioning could take advantage of exclusive user-defined subsets
(e.g. folders, but not necessarily tags), *however* the user may move an element to
another subset, which makes history tracking difficult. It may be better not to partition
on this type of thing, but either rely on full-content searches or on separate lists to
find elements by user-defined subset. If separate lists are used, it may be simpler to
make these simply caches and not try to synchronise them or protect their contents
from corruption.

On the other hand, it may make some sense to partition according to user-defined
subsets; for example a common use would be for an "inbox" folder, and it would be
desirable to allow its contents to be quickly found without having to scan all elements
(which may include tens of thousands of archived items).

Further: it is required that data can be partitioned according to some
user-defined criteria (e.g. folders) and that the current state of a partition
can be read and edited quickly even if the entire data-set or history is huge.

Rewriting: it would probably make sense if old entries are never removed from
the history log but can be "deleted" from some partition by a new log entry.

### Inter-partition integrity

Given that an element is moved from one partition to another, how can you
ensure it is neither lost nor duplicated during the move?

Option 1) record details of the move in both partitions. When loading either,
force loading of the other to check that the move was indeed recorded there
too. In the worst case this could force loading of all partitions, which
defeats the purpose of partitioning; note however that "loading" could be
short-cut to just "check the log".

Option 2) record details in the destination log first and possibly verify this,
then commit to the source log. Does not protect against duplication.

Option 3) admit to users of the library that moves between partitions are not
fully safeguarded.

### Partitions, files and identification

What is a *partition?* Is it (1) the elements and history stored within a
single file, or (2) the sub-set of elements which are stored in a single file
but including the full history in previous and possibly more recent files?

From the API perspective, it may sometimes be required to know which segments of
history are loaded, but for the most part the user will not be interested in
history thus the API will be simpler if a "partition" in the API refers to a
sub-set of element and ignores history.

There needs to be some file naming convention and way of referring to
partitions within the API; each element must be uniquely identified by the
(partition identifier, element identifier) pair.
Requirements:

*   There must be a map from elements to partitions which is deterministic and
    discoverable without loading the partition data or determining which
    elements exist.
*   It is required that a file clearly identifies which elements it may contain in
    either the file name or the header.
*   It is required that there be a global method of designing new partitions
    when required.
*   It is strongly preferred that the library have a set of user-defined
    classifiers on elements that it may use in a user-defined order to design
    new partitions, and that these classifiers provide sufficient granularity
    that partition sizes are never forced to be larger than desired.


Encryption
-------------

Possibly log-files should have integrated support for encryption. But what should be
encrypted? Obviously element contents, but what about time-stamps of changes
and information on the partitioning (which elements and time-frame)?

Is it advantageous to encrypt the contents of some parts of the file as opposed to
encrypting the whole file on disk? Possibly, since we want to be able to append data
to the file without rewriting the whole thing.

How should the encryption work? Store a symmetric key at the start of the file,
encrypted by some external key or some password?

How many people will want to encrypt the mail on disk anyway, and not simply
by whole-disk encryption?


Compression
--------------

Since a log should consist of a large number of short pieces of data, it seems to
make most sense if compression is done only at the level of whole files, the same
as with encryption.

However, since the intention is that log files can be extended bit-by-bit without
requiring rewriting the whole file, it may make sense that the file is divided into
"blocks" (e.g. the header, then a number of fixed-size blocks each containing log
entries until full), with each block compressed independently.


Structure
-----------

A log file needs to represent a set of elements, some of which may change over
time. It is the intention that the log may be written only by appending data and
modifying a fixed-length header.

Option 1: no index. Finding elements requires parsing the entire file; if read in
forward order element history is read oldest-to-newest.

Option 2: fixed size header accommodating a set number of elements; for each some
identifier (maybe just a number) and some address within the file would be stored.
The address would point to the latest log entry for the element; each log entry
would contain the address of the previous log entry. When the header runs out of
room for new elements a new log file must be started.


Compaction
----------------

It may well be desirable to "rewrite history" in a way that combines change
records in order to reduce the size of the historical record, possibly even
across partitions, in order to reduce disk space.

New log entries would be per-event; compaction could for example reduce this to
daily, weekly, monthly or annual entries. Entries added then deleted within the
time-frame of a single entry would be lost entirely. When entries become
sufficiently large, an entry may simply be a complete snapshot of all entries
in the partition instead of some kind of "diff" patch.

When compaction is done on one node, other nodes synchronising with it would
have four options: keep extra history locally (no change), re-do compaction
locally (expensive on the CPU), inherit compaction blindly (expensive download,
increased chance of corruption), or push local history to the remote node
(undo compaction, assuming history on local node is correct).

It would be desirable that two nodes performing compaction independently but
then synchronising would end up with the same result, even if compaction were
done in multiple stages on one node. There should be some fixed algorithm for
choosing which time-points to keep.


Evaluation of options
==============

Following on from the above, we evaluate possible options.


Requirements summary
------------------------------

One file stores a "partition" of full data set, potentially restricting both
vertically (time frame covered) and horizontally.

Files must store changes (potentially all changes within a partition).

Compression required _somewhere_. Checksums also required.

It must be possible to make changes by only changing the end of the file
and possibly the header.


Options for storing history in partition files
-----------------------------------------------------------

### Snapshot + changesets

Obviously it's undesirable to have to read all history to reconstruct the
current state, yet maintaining full history is desired. The simplest solution
is to "cache" the full state at various points in time into a _snapshot_ while
keeping a log of all changes.

The simplest way of implementing this would be that each file contains a
snapshot and changesets following the snapshot. This also meets my data-loss
requirements.

Open question: is it worth supporting snapshots at any point *after* the start
of a partition (i.e. file), i.e. allowing new snapshots without starting a new
file/partition?

### Running state

Another approach: maintain the latest state in memory, and write this out to
disk every so often as a "check point". On next load recover from the latest
check point plus log entries.

This requires more writes to disk but may achieve faster loads. There may be a
higher chance of corruption, but by using checksums and keeping multiple
checkpoints it _should_ be okay.

This could be an option _on top of_ snapshots stored at the start of new
partitions. Further, since there is an increased chance of corruption, I will
not pursue this option unless performance tests later show that it could be
useful.

### Add an index?

This is orthoganol to the above options. Should an index be kept?

For this: it allows reading a sub-set of messages without reading the entire
file. It could make reconstructing the latest state from a large log faster
(at least, on disks happy to do lots of seeking, and this is only really
advantageous for very large logs, which should probably be partitioned anyway).

Against: it means space has to be preallocated in the header, and either the
header expanded or a new partition started when that space is used up. I also
don't see much point since the system is designed only to hold small pieces
of data (and is intended itself to be an index of sorts).

### Support moving items?

If a message is moved into a different folder or under a different tag, should
this system support moving items in any other way than recreating them?

To collect all information on items matching a certain tag or path including
those not originally created with that tag/path, without collecting information
on all items (up to the point of deletion), and without having to re-read (do
a second pass), it will be required to list items in full if a tag is added or
path changed.

Suggestion: if the primary key (used to organise items between partitions)
changes, then list a "moved to ..." log entry in the old partition and a "moved
from ..." entry including the full state in the new partition (which may be the
same one). Possibly also list the full state when some application-specific
property of the data changes is true.


Appending new commits
----------------------

There is an issue with simply appending data to a file: if the operation fails,
it might corrupt the file. It is therefore worth looking at using multiple
files.

Each partition will thus have multiple files used to reconstruct the current
state as well as potential historical files. This might make it worth using a
sub-directory for each partition.

**Separate log files:** since log files will need to be updated more frequently
than snapshots and snapshots may be large, it makes sense to store them in
separate files.

**Snapshots:** a new snapshot may be written whenever log files are deemed
large enough to start fresh. The snapshot files need not be duplicated; in the
case that a snapshot file appears corrupt, the program may reconstruct the
state from a previous snapshot plus log files.

**Log files:** whichever log file(s) are required to reconstruct the current
state should not be altered. New log entries should either be written in new
log files or by altering a redundant log file; either way the new file should
contain all entries from the other log files. Reconstructing the new state
should read all log files involved then mark redundant files for deletion or
extension.

A **merge** could be done from multiple log files in the same location or by
pulling commits from some other source; either way the merge should proceed by
(1) reconstructing the latest state of each branch, (2) accepting changes which
do not conflict and somehow resolving those which do, (3) creating a special
"merge commit" which details which changes were rejected and any new changes,
and (4) writing all commits involved to the log, marking them with a branch
identifier so that reconstruction is possible.

What if a clone makes some more modifications before accepting the merge?

Dealing with **corruption:** if a snapshot is found to be corrupt, the program
should try to reproduce history up to that point, and if successful rewrite the
snapshot. If a commit is found to be corrupt, the program should try to find
another copy of that commit, either locally or in a clone of the repository.
When these strategies fail, things get complicated (user intervention, best
guesses or dropping data).


Commit contents
----------------------

What does a commit need to contain?

*   a pattern to indicate that this is the start of a commit
*   identification of parent commit (or commits in the case of a merge)
*   some time stamp
*   some mention of length and/or how many items are changed
*   a state checksum to verify the reconstructed state of the repository
*   some identifier
*   a checksum of the commit itself (must logically come last)

And, for each item mentioned in the commit:

*   an item identifier
*   a full copy of its new state OR
*   a patch OR
*   a mention of its deletion OR
*   a mention of it being moved to another partition, including where
*   finally, a checksum of the latest state (excepting deletion or moving)

In the fashion of git and several other DVCSs, the checksum of the commit might
as well also serve as the commit identifier.


Contents of items in the data set
-----------------

For the email application, we probably need key-value pairs so that changes
affecting different keys can be merged easily. At any rate, there needs to be
some way of merging conflicting changes (doing a three-way merge),

Option 1: always store the full state and allow an application-specific
algorithm to be used to merge conflicting new states.

Option 2: allow application-specific algorithms to generate patches and apply
them, along with some back-up handler if patching fails when merging.


Checksumming
--------------------

Of elements/items: may be useful to allow faster calculation of state checksums
but I see little other use.

Of file header, snapshot, and log entries: this allows detection of file corruption.

State checksums option 1: XOR of a checksum of each item. Simple, easy to
parallelise calculation, possible to calculate from a change log without full
data. Against: relies on strength of underlying checksum to prevent intentional
collisions (which *might* allow history alterations during merges).

State checksums option 2: checksum of concatenation of all data items. Possibly
more secure but requires full data set to calculate.
Reduces meta-data stored for each item in the set.

State checksums option 3: store a less secure checksum for each data item (e.g.
SHA-1 160-bit or even MD5 128-bit) then calculate a secure checksum of the
concatenation of each item's checksum. Possibly more secure than option 1, or
possibly less: one data item could be changed to something whose checksum
collides with that of the original without detection.

Note: options 2 and 3 can support parallelisation by calculating in a pyramid
fashion (e.g. contatenate 64 items and calculate checksum, then concatenate
64 of those checksums...).
